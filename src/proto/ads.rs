// *****************************************************************************
// PILS PLC client library
// Copyright (c) 2021 by the authors, see LICENSE
//
// This program is free software; you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation; either version 2 of the License, or (at your option) any later
// version.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more
// details.
//
// You should have received a copy of the GNU General Public License along with
// this program; if not, write to the Free Software Foundation, Inc.,
// 59 Temple Place, Suite 330, Boston, MA  02111-1307  USA
//
// Module authors:
//   Georg Brandl <g.brandl@fz-juelich.de>
//
// *****************************************************************************

use std::convert::TryInto;
use std::io::{Read, Write};
use std::mem::size_of;
use std::net::{IpAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use crate::{Error, Result};
use crate::proto::{CONNECT_TIMEOUT, Protocol, READ_TIMEOUT, WRITE_TIMEOUT};

use itertools::Itertools;
use regex::Regex;
use once_cell::sync::Lazy;
use zerocopy::{FromBytes, AsBytes};

const ADS_PORT: u16 = 0xBF02;
const UDP_PORT: u16 = 0xBF03;
const INDEXGROUP_M: u32 = 0x4020;

// ADS commands.
const ADS_DEVINFO: u16 = 1;
const ADS_READ: u16    = 2;
const ADS_WRITE: u16   = 3;

static ADS_ADDR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"ads://(.+?)/(\d+.\d+.\d+.\d+(.\d+.\d+)?):(\d+)$")
        .expect("invalid regex")
});
const ADS_ADDR_FMT: &str = "ads://host[:port]/amsnetid:amsport";

pub struct AdsProto {
    host: String,
    port: u16,
    target_netid: [u8; 6],
    target_port: u16,
    source_netid: [u8; 6],
    invoke_id: u32,
    tried_route: bool,
    socket: Option<TcpStream>,
}

impl AdsProto {
    pub fn new(addr: &str) -> Result<Self> {
        let err0 = || Error::InvalidAddress(ADS_ADDR_FMT);
        let err1 = |_| Error::InvalidAddress(ADS_ADDR_FMT);
        let caps = ADS_ADDR_RE.captures(addr).ok_or_else(err0)?;
        let host = &caps[1];
        let (host, port) = if host.contains(':') {
            let (h1, p1) = host.splitn(2, ':').collect_tuple().expect("split");
            (h1.into(), p1.parse().map_err(err1)?)
        } else {
            (host.into(), ADS_PORT)
        };

        let octets = caps[2].split('.').map(|p| p.parse().map_err(err1))
                                       .collect::<Result<Vec<_>>>()?;
        let netid = if octets.len() == 6 {
            octets.try_into().unwrap()
        } else if octets.len() == 4 {
            [octets[0], octets[1], octets[2], octets[3], 1, 1]
        } else {
            return Err(err0());
        };
        let amsport = caps[4].parse().map_err(err1)?;

        Ok(Self {
            host, port,
            target_netid: netid,
            target_port: amsport,
            source_netid: [0; 6],
            invoke_id: 0,
            tried_route: false,
            socket: None,
        })
    }

    fn communicate(&mut self, cmd: u16, data_in: &[u8], data_in_2: &[u8],
                   data_out: &mut [u8], data_out_2: &mut [u8]) -> Result<()> {
        log::debug!("communicate: {}+{} -> {}+{} bytes", data_in.len(), data_in_2.len(),
                    data_out.len(), data_out_2.len());
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let header_in = AdsHeader {
            reserved: 0,
            length: (size_of::<AdsHeader>() - 6 + data_in.len()) as u32,
            target_netid: self.target_netid,
            target_port: self.target_port,
            source_netid: self.source_netid,
            source_port: 800,
            command_id: cmd,
            state_flags: 4,
            data_length: data_in.len() as u32,
            error_code: 0,
            invoke_id: self.invoke_id
        };
        let mut reply = AdsReply::new_zeroed();
        if let Some(sock) = self.socket.as_mut() {
            sock.write_all(header_in.as_bytes())?;
            sock.write_all(data_in)?;
            if !data_in_2.is_empty() {
                sock.write_all(data_in_2)?;
            }
            sock.read_exact(reply.as_bytes_mut())?;
            let AdsReply { header: header_out, result } = reply;
            if header_out.state_flags != 5 {
                return Err(Error::ADS("unexpected state flags", header_out.state_flags as u32));
            }
            if header_out.data_length as usize != data_out.len() + data_out_2.len() + 4 {
                return Err(Error::ADS("unexpected data length", header_out.data_length));
            }
            if header_out.error_code != 0 {
                return find_ads_error(header_out.error_code);
            }
            if header_out.invoke_id != header_in.invoke_id {
                return Err(Error::ADS("invoke ID not matching", header_out.invoke_id));
            }
            if result != 0 {
                return find_ads_error(result);
            }
            sock.read_exact(data_out)?;
            if !data_out_2.is_empty() {
                sock.read_exact(data_out_2)?;
            }
        }
        Ok(())
    }

    fn set_route(&mut self) -> Result<()> {
        let netid = self.source_netid;
        let myhost = format!("{}.{}.{}.{}", netid[0], netid[1], netid[2], netid[3]);
        let routename = format!("zapf-{}", myhost);
        let udpsock = UdpSocket::bind("127.0.0.1:0")?;
        let mut msg = UdpRouteMessage {
            magic: UDP_MAGIC,
            padding: 0,
            operation: UDP_ADD_ROUTE,
            source_netid: self.source_netid,
            source_port: 800,
            num_items: 5,
            desig_route: UDP_ROUTENAME,
            strlen_route: 25,
            str_route: to_array(&routename),
            desig_netid: UDP_NETID,
            strlen_netid: 6,
            str_netid: netid,
            desig_user: UDP_USERNAME,
            strlen_user: 14,
            str_user: to_array("Administrator"),
            desig_pass: UDP_PASSWORD,
            strlen_pass: 2,
            str_pass: [0, 0],
            desig_host: UDP_HOST,
            strlen_host: 16,
            str_host: to_array(&myhost),
        };
        for &&pass in &[b"\x00\x00", b"1\x00"] {
            msg.str_pass = pass;
            udpsock.send_to(msg.as_bytes(), (self.host.as_str(), UDP_PORT))?;
        }
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    }
}

fn to_array<const N: usize>(s: &str) -> [u8; N] {
    let mut array = [0; N];
    for (i, ch) in s.chars().enumerate().take(N-1) {
        array[i] = ch as u8;
    }
    array
}

impl Protocol for AdsProto {
    fn get_offsets(&self) -> &[usize] {
        &[0]
    }

    fn set_offset(&mut self, _: usize) { }

    fn connect(&mut self) -> Result<()> {
        let sock = {
            // Call to connect_timeout needs to be done on a single address
            let mut socket_addrs = (self.host.as_str(), self.port).to_socket_addrs()?;
            TcpStream::connect_timeout(&socket_addrs.next().unwrap(), CONNECT_TIMEOUT)?
        };

        sock.set_read_timeout(Some(READ_TIMEOUT))?;
        sock.set_write_timeout(Some(WRITE_TIMEOUT))?;
        sock.set_nodelay(true)?;

        let my_addr = sock.local_addr()?.ip();
        self.source_netid = if let IpAddr::V4(ip) = my_addr {
            let [a, b, c, d] = ip.octets();
            [a, b, c, d, 1, 1]
        } else {
            [127, 0, 0, 1, 1, 1]
        };

        self.socket = Some(sock);
        let mut devinfo = AdsDevInfo::new_zeroed();

        let result = self.communicate(ADS_DEVINFO, &[], &[], &mut [], devinfo.as_bytes_mut());
        if let Err(Error::IO(e)) = &result {
            // If we get a closed connection immediately, the route is not set
            // up correctly.  Try to fix that by setting a route via UDP.  If
            // the target TCP port is not default, we are not talking directly
            // to TwinCAT, so don't even try in that case.
            if e.kind() == std::io::ErrorKind::UnexpectedEof &&
                !self.tried_route &&
                self.port == ADS_PORT
            {
                log::warn!("connection aborted, trying to set a route...");
                self.tried_route = true;
                let _ = self.set_route();
                return self.connect();
            }
        }
        result?;

        let name = devinfo.devname.iter().take_while(|&&ch| ch > 0)
                                         .map(|&ch| ch as char).collect::<String>();
        log::info!("connected to {} {}.{}.{}", name,
                   devinfo.major, devinfo.minor, devinfo.version + 0);

        Ok(())
    }

    fn disconnect(&mut self) {
        self.socket = None;
    }

    fn read_into(&mut self, addr: usize, data: &mut [u8]) -> Result<()> {
        if self.socket.is_none() {
            self.reconnect()?;
        }
        let payload = AdsDataAddr {
            indexgroup: INDEXGROUP_M,
            address: addr as u32,
            length: data.len() as u32,
        };
        let mut len = [0; 4];
        self.communicate(ADS_READ, payload.as_bytes(), &[], &mut len, data)
            .map_err(|e| {
                self.disconnect();
                log::error!("during ADS read: {}", e);
                Error::Wrapped(Box::new(e), "read")
            })
    }

    fn write(&mut self, addr: usize, data: &[u8]) -> Result<()> {
        if self.socket.is_none() {
            self.reconnect()?;
        }
        let payload = AdsDataAddr {
            indexgroup: INDEXGROUP_M,
            address: addr as u32,
            length: data.len() as u32,
        };
        self.communicate(ADS_WRITE, payload.as_bytes(), &[], &mut [], &mut [])
            .map_err(|e| {
                self.disconnect();
                log::error!("during ADS read: {}", e);
                Error::Wrapped(Box::new(e), "read")
            })
    }
}


#[repr(C, packed)]
#[derive(FromBytes, AsBytes)]
struct AdsHeader {
    reserved:     u16,
    length:       u32,
    target_netid: [u8; 6],
    target_port:  u16,
    source_netid: [u8; 6],
    source_port:  u16,
    command_id:   u16,
    state_flags:  u16,
    data_length:  u32,
    error_code:   u32,
    invoke_id:    u32,
}

#[repr(C, packed)]
#[derive(FromBytes, AsBytes)]
struct AdsReply {
    header: AdsHeader,
    result: u32,
}

#[repr(C, packed)]
#[derive(AsBytes)]
struct AdsDataAddr {
    indexgroup: u32,
    address: u32,
    length: u32,
}

#[repr(C, packed)]
#[derive(FromBytes, AsBytes)]
struct AdsDevInfo {
    major:   u8,
    minor:   u8,
    version: u16,
    devname: [u8; 16],
}

#[repr(C, packed)]
#[derive(AsBytes)]
struct UdpRouteMessage {
    magic:        u32,
    padding:      u32,
    operation:    u32,
    source_netid: [u8; 6],
    source_port:  u16,
    num_items:    u32,
    desig_route:  u16,
    strlen_route: u16,
    str_route:    [u8; 25],
    desig_netid:  u16,
    strlen_netid: u16,
    str_netid:    [u8; 6],
    desig_user:   u16,
    strlen_user:  u16,
    str_user:     [u8; 14],
    desig_pass:   u16,
    strlen_pass:  u16,
    str_pass:     [u8; 2],
    desig_host:   u16,
    strlen_host:  u16,
    str_host:     [u8; 16],
}

// UDP magic header number.
const UDP_MAGIC: u32 = 0x71146603;
// UDP packet operations and data designators.
const UDP_ADD_ROUTE: u32 = 6;
const UDP_PASSWORD: u16 = 2;
const UDP_HOST: u16 = 5;
const UDP_NETID: u16 = 7;
const UDP_ROUTENAME: u16 = 12;
const UDP_USERNAME: u16 = 13;


// https://infosys.beckhoff.com/english.php?content=../content/1033/tc3_ads_intro_howto/374277003.html&id=2736996179007627436
const ADS_ERRORS: &[(u32, &str)] = &[
    (0x001, "Internal error"),
    (0x002, "No real-time"),
    (0x003, "Allocation locked - memory error"),
    (0x004, "Mailbox full - ADS message could not be sent"),
    (0x005, "Wrong receive HMSG"),
    (0x006, "Target port not found, possibly ADS server not started"),
    (0x007, "Target machine not found, possibly missing ADS routes"),
    (0x008, "Unknown command ID"),
    (0x009, "Invalid task ID"),
    (0x00A, "No IO"),
    (0x00B, "Unknown AMS command"),
    (0x00C, "Win32 error"),
    (0x00D, "Port not connected"),
    (0x00E, "Invalid AMS length"),
    (0x00F, "Invalid AMS NetID"),
    (0x010, "Low installation level"),
    (0x011, "No debugging available"),
    (0x012, "Port disabled - system service not started"),
    (0x013, "Port already connected"),
    (0x014, "AMS Sync Win32 error"),
    (0x015, "AMS Sync timeout"),
    (0x016, "AMS Sync error"),
    (0x017, "AMS Sync no index map"),
    (0x018, "Invalid AMS port"),
    (0x019, "No memory"),
    (0x01A, "TCP send error"),
    (0x01B, "Host unreachable"),
    (0x01C, "Invalid AMS fragment"),
    (0x01D, "TLS send error - secure ADS connection failed"),
    (0x01E, "Access denied - secure ADS access denied"),

    (0x500, "Router: no locked memory"),
    (0x501, "Router: memory size could not be changed"),
    (0x502, "Router: mailbox full"),
    (0x503, "Router: debug mailbox full"),
    (0x504, "Router: port type is unknown"),
    (0x505, "Router is not initialized"),
    (0x506, "Router: desired port number is already assigned"),
    (0x507, "Router: port not registered"),
    (0x508, "Router: maximum number of ports reached"),
    (0x509, "Router: port is invalid"),
    (0x50A, "Router is not active"),
    (0x50B, "Router: mailbox full for fragmented messages"),
    (0x50C, "Router: fragment timeout occurred"),
    (0x50D, "Router: port removed"),

    (0x700, "General device error"),
    (0x701, "Service is not supported by server"),
    (0x702, "Invalid index group"),
    (0x703, "Invalid index offset"),
    (0x704, "Reading/writing not permitted"),
    (0x705, "Parameter size not correct"),
    (0x706, "Invalid parameter value(s)"),
    (0x707, "Device is not in a ready state"),
    (0x708, "Device is busy"),
    (0x709, "Invalid OS context -> use multi-task data access"),
    (0x70A, "Out of memory"),
    (0x70B, "Invalid parameter value(s)"),
    (0x70C, "Not found (files, ...)"),
    (0x70D, "Syntax error in command or file"),
    (0x70E, "Objects do not match"),
    (0x70F, "Object already exists"),
    (0x710, "Symbol not found"),
    (0x711, "Symbol version invalid -> create a new handle"),
    (0x712, "Server is in an invalid state"),
    (0x713, "AdsTransMode not supported"),
    (0x714, "Notification handle is invalid"),
    (0x715, "Notification client not registered"),
    (0x716, "No more notification handles"),
    (0x717, "Notification size too large"),
    (0x718, "Device not initialized"),
    (0x719, "Device has a timeout"),
    (0x71A, "Query interface failed"),
    (0x71B, "Wrong interface required"),
    (0x71C, "Class ID is invalid"),
    (0x71D, "Object ID is invalid"),
    (0x71E, "Request is pending"),
    (0x71F, "Request is aborted"),
    (0x720, "Signal warning"),
    (0x721, "Invalid array index"),
    (0x722, "Symbol not active -> release handle and try again"),
    (0x723, "Access denied"),
    (0x724, "No license found -> activate license"),
    (0x725, "License expired"),
    (0x726, "License exceeded"),
    (0x727, "License invalid"),
    (0x728, "Invalid system ID in license"),
    (0x729, "License not time limited"),
    (0x72A, "License issue time in the future"),
    (0x72B, "License time period too long"),
    (0x72C, "Exception in device specific code -> check each device"),
    (0x72D, "License file read twice"),
    (0x72E, "Invalid signature"),
    (0x72F, "Invalid public key certificate"),
    (0x730, "Public key not known from OEM"),
    (0x731, "License not valid for this system ID"),
    (0x732, "Demo license prohibited"),
    (0x733, "Invalid function ID"),
    (0x734, "Outside the valid range"),
    (0x735, "Invalid alignment"),
    (0x736, "Invalid platform level"),
    (0x737, "Context - forward to passive level"),
    (0x738, "Content - forward to dispatch level"),
    (0x739, "Context - forward to real-time"),

    (0x740, "General client error"),
    (0x741, "Invalid parameter at service"),
    (0x742, "Polling list is empty"),
    (0x743, "Var connection already in use"),
    (0x744, "Invoke ID in use"),
    (0x745, "Timeout elapsed -> check route setting"),
    (0x746, "Error in Win32 subsystem"),
    (0x747, "Invalid client timeout value"),
    (0x748, "ADS port not opened"),
    (0x749, "No AMS address"),
    (0x750, "Internal error in ADS sync"),
    (0x751, "Hash table overflow"),
    (0x752, "Key not found in hash"),
    (0x753, "No more symbols in cache"),
    (0x754, "Invalid response received"),
    (0x755, "Sync port is locked"),

    (0x1000, "Internal error in real-time system"),
    (0x1001, "Timer value not valid"),
    (0x1002, "Task pointer has invalid value 0"),
    (0x1003, "Stack pointer has invalid value 0"),
    (0x1004, "Requested task priority already assigned"),
    (0x1005, "No free Task Control Block"),
    (0x1006, "No free semaphores"),
    (0x1007, "No free space in the queue"),
    (0x100D, "External sync interrupt already applied"),
    (0x100E, "No external sync interrupt applied"),
    (0x100F, "External sync interrupt application failed"),
    (0x1010, "Call of service function in wrong context"),
    (0x1017, "Intel VT-x not supported"),
    (0x1018, "Intel VT-x not enabled in BIOS"),
    (0x1019, "Missing function in Intel VT-x"),
    (0x101A, "Activation of Intel VT-x failed"),
];

fn find_ads_error<T>(err: u32) -> Result<T> {
    match ADS_ERRORS.binary_search_by_key(&err, |e| e.0) {
        Ok(idx) => Err(Error::ADS(ADS_ERRORS[idx].1, err)),
        Err(_) => Err(Error::ADS("Unknown error code", err))
    }
}

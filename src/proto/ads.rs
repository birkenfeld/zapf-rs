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

use itertools::Itertools;
use regex::Regex;
use once_cell::sync::Lazy;

use crate::{Error, Result};
use crate::proto::{CONNECT_TIMEOUT, Protocol, READ_TIMEOUT, WRITE_TIMEOUT};

static ADS_ADDR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"ads://(.+?)/(\d+.\d+.\d+.\d+(.\d+.\d+)?):(\d+)$")
        .expect("invalid regex")
});
const ADS_ADDR_FMT: &str = "ads://host[:port]/amsnetid:amsport";

pub struct AdsProto {
    host: String,
    port: u16,
    target: ads::AmsAddr,
    tried_route: bool,
    client: Option<ads::Client>,
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
            (host.into(), ads::PORT)
        };

        let netid = caps[2].parse().map_err(|_| err0())?;
        let amsport = caps[4].parse().map_err(err1)?;

        Ok(Self {
            host, port,
            target: ads::AmsAddr::new(netid, amsport),
            tried_route: false,
            client: None,
        })
    }

    fn set_route(&self, src: ads::AmsNetId) {
        let myhost = format!("{}.{}.{}.{}", src.0[0], src.0[1], src.0[2], src.0[3]);
        let routename = format!("zapf-{}", myhost);
        for pass in &["", "1"] {
            if ads::udp::add_route((self.host.as_str(), ads::UDP_PORT),
                                   self.target.netid(),
                                   &myhost, Some(&routename), None, Some(pass),
                                   false).is_ok() {
                break;
            }
        }
    }
}

impl Protocol for AdsProto {
    fn get_offsets() -> &'static [usize] {
        &[0]
    }

    fn set_offset(&mut self, _: usize) { }

    fn connect(&mut self) -> Result<()> {
        let timeouts = ads::Timeouts {
            connect: Some(CONNECT_TIMEOUT),
            write: Some(WRITE_TIMEOUT),
            read: Some(READ_TIMEOUT),
        };
        let client = ads::Client::new((self.host.as_str(), self.port), timeouts, None)?;

        let info = match client.device(self.target).get_info() {
            Ok(info) => info,
            Err(ads::Error::Io(_, ioe)) if
                ioe.kind() == std::io::ErrorKind::UnexpectedEof &&
                !self.tried_route &&
                self.port == ads::PORT =>
            {
                log::warn!("connection aborted, trying to set a route...");
                self.tried_route = true;
                self.set_route(client.source().netid());
                return self.connect();
            }
            Err(e) => Err(e)?,
        };

        self.client = Some(client);
        log::info!("connected to {} {}.{}.{}", info.name,
                   info.major, info.minor, info.version);
        Ok(())
    }

    fn disconnect(&mut self) {
        self.client = None;
    }

    fn read_into(&mut self, addr: usize, data: &mut [u8]) -> Result<()> {
        if self.client.is_none() {
            self.reconnect()?;
        }
        let device = self.client.as_ref().unwrap().device(self.target);
        device.read_exact(ads::index::PLC_RW_M, addr as u32, data).map_err(Into::into)
    }

    fn write(&mut self, addr: usize, data: &[u8]) -> Result<()> {
        if self.client.is_none() {
            self.reconnect()?;
        }
        let device = self.client.as_ref().unwrap().device(self.target);
        device.write(ads::index::PLC_RW_M, addr as u32, data).map_err(Into::into)
    }
}

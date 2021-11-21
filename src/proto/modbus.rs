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

use crate::{Error, Result};
use crate::proto::{CONNECT_TIMEOUT, Protocol, READ_TIMEOUT, WRITE_TIMEOUT};

use modbus::{Client, tcp::Config};
use regex::Regex;
use once_cell::sync::Lazy;

static MB_ADDR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"modbus://(.+?)(?::(\d+))?(?:/(\d+)?)?$")
        .expect("invalid regex")
});
const MB_ADDR_FMT: &str = "modbus://host[:port]/slave";

const MB_PORT: u16 = 502;

pub struct ModbusProto {
    host: String,
    config: Config,
    client: Option<modbus::Transport>,
    offset: usize,
}

impl ModbusProto {
    pub fn new(addr: &str) -> Result<Self> {
        let err0 = || Error::InvalidAddress(MB_ADDR_FMT);
        let err1 = |_| Error::InvalidAddress(MB_ADDR_FMT);
        let caps = MB_ADDR_RE.captures(addr).ok_or_else(err0)?;
        let host = caps[1].into();
        let port = if let Some(port) = caps.get(2) {
            port.as_str().parse().map_err(err1)?
        } else {
            MB_PORT
        };
        let slave = if let Some(slave) = caps.get(3) {
            slave.as_str().parse().map_err(err1)?
        } else {
            0
        };
        let config = Config {
            tcp_port: port,
            modbus_uid: slave,
            tcp_connect_timeout: Some(CONNECT_TIMEOUT),
            tcp_read_timeout: Some(READ_TIMEOUT),
            tcp_write_timeout: Some(WRITE_TIMEOUT),
        };

        Ok(Self { host, config, offset: 0, client: None })
    }

    fn convert_addr(&self, addr: usize) -> Result<u16> {
        ((self.offset + addr) / 2)
            .try_into()
            .map_err(|_| modbus::Error::InvalidData(
                modbus::Reason::Custom("Address too big".into())).into())
    }
}

impl Protocol for ModbusProto {
    fn get_offsets() -> &'static [usize] {
        &[0, 0x6000, 0x8000]
    }

    fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
    }

    fn connect(&mut self) -> Result<()> {
        let client = modbus::Transport::new_with_cfg(&self.host, self.config)?;

        self.client = Some(client);

        log::info!("connected to {}", self.host);
        Ok(())
    }

    fn disconnect(&mut self) {
        self.client = None;
    }

    fn read_into(&mut self, addr: usize, data: &mut [u8]) -> Result<()> {
        if self.client.is_none() {
            self.reconnect()?;
        }
        let mut addr = self.convert_addr(addr)?;
        let client = self.client.as_mut().unwrap();
        // TODO split requests if too large data is requested
        let mut length = data.len();
        let mut offset = 0;
        while length > 0 {
            let plen = length.min(250);
            match client.read_holding_registers(addr, (plen / 2) as u16) {
                Ok(regs) => {
                    for (i, reg) in regs.into_iter().enumerate() {
                        data[offset + 2*i] = reg as u8;
                        data[offset + 2*i + 1] = (reg >> 8) as u8;
                    }
                }
                Err(modbus::Error::Io(ioe)) => {
                    self.disconnect();
                    log::error!("during Modbus read: {}", ioe);
                    return Err(Error::Wrapped(Box::new(modbus::Error::Io(ioe).into()), "read"));
                }
                Err(e) => return Err(e.into())
            }
            length -= plen;
            offset += plen;
            addr += (plen / 2) as u16;
        }
        Ok(())
    }

    fn write(&mut self, addr: usize, data: &[u8]) -> Result<()> {
        if self.client.is_none() {
            self.reconnect()?;
        }
        let addr = self.convert_addr(addr)?;
        let client = self.client.as_mut().unwrap();
        let mut regs = vec![0; data.len() / 2];
        for (i, reg) in regs.iter_mut().enumerate() {
            *reg = data[2*i] as u16 | (data[2*i + 1] as u16) << 8;
        }
        client.write_multiple_registers(addr, &regs)
              .map_err(|e| if let modbus::Error::Io(ioe) = e {
                  self.disconnect();
                  log::error!("during Modbus write: {}", ioe);
                  Error::Wrapped(Box::new(modbus::Error::Io(ioe).into()), "write")
              } else {
                  e.into()
              })
    }
}

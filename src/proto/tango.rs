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

use crate::{Error, Result};
use crate::proto::Protocol;

use tango_client::{CommandData, DeviceProxy};
use regex::Regex;
use once_cell::sync::Lazy;

static TG_ADDR_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"tango://([\w.-]+:[\d]+/)?([\w-]+/){2}[\w-]+(#dbase=(no|yes))?$")
        .expect("invalid regex")
});
const TG_ADDR_FMT: &str = "tango://[database:port/]domain/family/member[#dbase=no]";

pub struct TangoProto {
    tango_dev: String,
    device: Option<DeviceProxy>,
    offset: usize,
}

impl TangoProto {
    pub fn new(addr: &str) -> Result<Self> {
        if !TG_ADDR_RE.is_match(addr) {
            return Err(Error::InvalidAddress(TG_ADDR_FMT));
        }
        Ok(Self { tango_dev: addr.into(), offset: 0, device: None })
    }
}

impl Protocol for TangoProto {
    fn get_offsets() -> &'static [usize] {
        &[0]
    }

    fn set_offset(&mut self, offset: usize) {
        self.offset = offset;
    }

    fn connect(&mut self) -> Result<()> {
        let mut device = DeviceProxy::new(&self.tango_dev)?;
        // check that the device is actually running
        let _state = device.command_inout("State", CommandData::Void)?;

        // check which interface we're dealing with
        if !(device.command_query("ReadInputBytes").is_ok() &&
             device.command_query("WriteOutputBytes").is_ok())
        {
            return Err(Error::TangoProto("Device has invalid interface"));
        }

        self.device = Some(device);

        log::info!("connected to {}", self.tango_dev);
        Ok(())
    }

    fn disconnect(&mut self) {
        self.device = None;
    }

    fn read_into(&mut self, addr: usize, data: &mut [u8]) -> Result<()> {
        if self.device.is_none() {
            self.reconnect()?;
        }
        let arg = vec![addr as u32, data.len() as u32];
        let device = self.device.as_mut().unwrap();
        // TODO: log + wrap errors
        let result = device.command_inout("ReadInputBytes",
                                          CommandData::ULongArray(arg))?;
        if let CommandData::CharArray(res) = result {
            if res.len() == data.len() {
                data.copy_from_slice(&res);
            }
            return Ok(());
        }
        return Err(Error::TangoProto("Invalid data type or length returned"));
    }

    fn write(&mut self, addr: usize, data: &[u8]) -> Result<()> {
        if self.device.is_none() {
            self.reconnect()?;
        }
        let mut arg = vec![addr as u32];
        arg.extend(data.iter().map(|&b| b as u32));
        let device = self.device.as_mut().unwrap();
        // TODO: log + wrap errors
        device.command_inout("WriteOutputBytes", CommandData::ULongArray(arg))?;
        Ok(())
    }
}

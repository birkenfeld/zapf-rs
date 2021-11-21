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

pub mod ads;
pub mod modbus;
#[cfg(feature = "tango_client")]
pub mod tango;

use std::time::Duration;

use crate::Result;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const READ_TIMEOUT: Duration = Duration::from_secs(1);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(1);

pub trait Protocol {
    fn connect(&mut self) -> Result<()>;
    fn disconnect(&mut self);
    fn reconnect(&mut self) -> Result<()> {
        self.connect()
    }

    fn read_into(&mut self, addr: usize, data: &mut [u8]) -> Result<()>;
    fn write(&mut self, addr: usize, data: &[u8]) -> Result<()>;

    fn read(&mut self, addr: usize, length: usize) -> Result<Vec<u8>> {
        let mut vec = vec![0; length];
        self.read_into(addr, &mut vec)?;
        Ok(vec)
    }

    fn get_offsets() -> &'static [usize];
    fn set_offset(&mut self, offset: usize);
}

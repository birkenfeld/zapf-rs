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

use zerocopy::AsBytes;

use crate::{Error, Result};
use crate::proto::Protocol;

pub enum Magic {
    M2015_02,
    M2021_09,
}

pub struct Io<P> {
    magic: Magic,
    cache: Cache,
    proto: P,
}

impl<P: Protocol> Io<P> {
    pub fn new(mut proto: P) -> Result<Self> {
        proto.connect()?;
        let cache = Cache {};
        let magic = detect_magic(&mut proto)?;
        Ok(Self { magic, cache, proto })
    }
}


fn detect_magic<P: Protocol>(proto: &mut P) -> Result<Magic> {
    let mut magic = 0f32;
    for &offset in P::get_offsets() {
        if proto.read_into(offset, magic.as_bytes_mut()).is_ok() {
            if magic >= 2015. && magic <= 2045. {
                if magic >= 2015.01 && magic <= 2015.03 {
                    return Ok(Magic::M2015_02);
                }
                if magic >= 2021.08 && magic <= 2021.10 {
                    return Ok(Magic::M2021_09);
                }
                return Err(Error::PLC(format!("Magic {} not supported", magic)));
            }
        }
    }
    Err(Error::PLC(format!("No supported magic or offset found")))
}

struct Cache {}

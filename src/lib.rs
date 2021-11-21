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

pub mod io;
pub mod proto;

use thiserror::Error;


#[derive(Debug, Error)]
pub enum Error {
    // address format specification error
    #[error("invalid address, must be {0}")]
    InvalidAddress(&'static str),
    // general IO error
    #[error(transparent)]
    IO(#[from] std::io::Error),

    // ADS specific error code
    #[error("ADS error: {0}")]
    ADS(#[from] ads::Error),

    // Modbus specific error code
    #[error("Modbus error: {0}")]
    Modbus(#[from] modbus::Error),

    // Exception from Tango
    #[cfg(feature = "tango_client")]
    #[error("Tango error: {0}")]
    Tango(#[from] tango_client::TangoError),
    // Tango related other error
    #[cfg(feature = "tango_client")]
    #[error("Tango error: {0}")]
    TangoProto(&'static str),

    // Zapf error with annotation
    #[error("during {1}: {0}")]
    Wrapped(#[source] Box<Error>, &'static str),

    #[error("PLC error: {0}")]
    PLC(String),

    // #[error(transparent)]
    // Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

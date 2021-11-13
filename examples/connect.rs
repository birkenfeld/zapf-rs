use zapf::{self, proto::Protocol};

fn main() -> zapf::Result<()> {
    env_logger::Builder::new().parse_filters("=debug").init();

    // let mut proto = zapf::proto::ads::AdsProto::new("ads://127.0.0.1/5.53.35.202.1.1:851")?;
    let mut proto = zapf::proto::modbus::ModbusProto::new("modbus://127.0.0.1:5002/0")?;
    proto.connect()?;
    let _ = dbg!(proto.read(0x6000, 4));

    Ok(())
}

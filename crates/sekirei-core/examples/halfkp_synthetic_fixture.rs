//! Export a deterministic HalfKP fixture for correctness and CPU diagnostics.
//! This network is synthetic and carries no playing-strength evidence.

use sekirei_core::halfkp::HalfKpNetwork;
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let output = args
        .next()
        .ok_or("usage: halfkp_synthetic_fixture OUTPUT [SEED]")?;
    let seed: u64 = args.next().map_or(Ok(20261014), |value| value.parse())?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let network = HalfKpNetwork::random(seed);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&network.to_bytes())?;
    Ok(())
}

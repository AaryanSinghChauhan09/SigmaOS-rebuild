//! SigmaOS component binary: rebuild

use sigmaos_rebuild::Component;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Component::new().operate()?;
    println!("sigmaos-rebuild: ok");
    Ok(())
}

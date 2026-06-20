use std::error::Error;

use radio_cat_rs::list_serial_ports;

fn main() -> Result<(), Box<dyn Error>> {
    for entry in list_serial_ports()? {
        println!("{entry}");
    }
    Ok(())
}

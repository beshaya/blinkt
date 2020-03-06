extern crate blinkt;
extern crate rand;

use rand::Rng;
use std::thread;
use std::time::Duration;

use blinkt::Blinkt;

fn main() {
    let mut rng = rand::thread_rng();
    let pixels = 100;
    let mut blinkt = Blinkt::with_spi(8_000_000, pixels).unwrap();

    blinkt.set_all_pixels_brightness(0.1);

    loop {
        for n in 0..8 {
            blinkt.set_pixel(n, rng.gen::<u8>(), rng.gen::<u8>(), rng.gen::<u8>());
        }
        blinkt.show().unwrap();

        thread::sleep(Duration::from_millis(50));
    }
}

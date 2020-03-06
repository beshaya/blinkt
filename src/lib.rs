// Copyright (c) 2016-2018 Rene van der Meer
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL
// THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

//! A Rust library that provides an interface for the Pimoroni Blinkt!, and any
//! similar APA102 or SK9822 LED strips or boards, on a Raspberry Pi. The library
//! supports bitbanging mode on any GPIO pins, and hardware SPI mode on GPIO 10
//! (physical pin 19) for data, and GPIO 11 (physical pin 23) for clock.
//!
//! For bitbanging mode, Blinkt gains access to the BCM283x GPIO peripheral either
//! through `/dev/gpiomem` or `/dev/mem`. Hardware SPI mode is controlled
//! through `/dev/spidev0.0`.
//!
//! Both the original APA102 and the SK9822 clone are supported. The APA102 RGB
//! LED/driver ICs are referred to as pixels throughout the code and documentation.
//!
//! Each pixel has a red, green and blue LED with possible values between 0-255.
//! Additionally, the overall brightness of each pixel can be set to 0.0-1.0, which
//! is converted to a 5-bit value.
//!
//! Blinkt stores all color and brightness changes in a local buffer. Use
//! `show()` to send the buffered values to the pixels.
//!
//! By default, all pixels are cleared when Blinkt goes out of
//! scope. Use `set_clear_on_drop(false)` to disable this behavior. Note that
//! drop methods aren't called when a program is abnormally terminated (for
//! instance when a SIGINT isn't caught).
//!
//! # Examples
//!
//! ### Blinkt! board
//!
//! A complete example that cycles all pixels on a Blinkt! board through red, green
//! and blue.
//!
//! ```rust,no_run
//! extern crate blinkt;
//!
//! use std::{thread, mem};
//! use std::time::Duration;
//!
//! use blinkt::Blinkt;
//!
//! fn main() {
//!     let mut blinkt = Blinkt::new().unwrap();
//!     let (red, green, blue) = (&mut 255, &mut 0, &mut 0);
//!
//!     loop {
//!         blinkt.set_all_pixels(*red, *green, *blue);
//!         blinkt.show().unwrap();
//!
//!         thread::sleep(Duration::from_millis(250));
//!
//!         mem::swap(red, green);
//!         mem::swap(red, blue);
//!     }
//! }
//! ```
//!
//! ### APA102 or SK9822 LED strip
//!
//! The recommended way to control an LED strip is to use the hardware SPI
//! interface through `Blinkt::with_spi()`, with the data line connected to GPIO 10
//! (physical pin 19), and clock on GPIO 11 (physical pin 23).
//!
//! ```rust,no_run
//! # extern crate blinkt;
//! #
//! # use blinkt::Blinkt;
//! #
//! # fn main() {
//! let mut blinkt = Blinkt::with_spi(16_000_000, 144).unwrap();
//! # }
//! ```
//!
//! Alternatively, you can use the bitbanging mode through `Blinkt::with_settings()`
//! to connect the LED strip to any available GPIO pins. However, this is less reliable
//! than using the hardware SPI interface, and may cause issues on longer strips.
//!
//! ```rust,no_run
//! # extern crate blinkt;
//! #
//! # use blinkt::Blinkt;
//! #
//! # fn main() {
//! let mut blinkt = Blinkt::with_settings(23, 24, 8).unwrap();
//! # }
//! ```

// Used by rustdoc to link other crates to blinkt's docs
#![doc(html_root_url = "https://docs.rs/blinkt/0.5.0")]
// Needed for the quick_error! macro
#![recursion_limit = "128"]

#[macro_use]
extern crate quick_error;
extern crate rppal;

use std::{io, result};

use rppal::gpio::{Gpio, Level, Mode};
use rppal::spi;

pub use rppal::gpio::Error as GpioError;
pub use rppal::spi::Error as SpiError;

// Default values for the Pimoroni Blinkt! board using BCM GPIO pin numbers
const DAT: u8 = 23;
const CLK: u8 = 24;
const NUM_PIXELS: usize = 8;

const DEFAULT_BRIGHTNESS: u8 = 7;

// Maximum transmission size of rppal::spi by default.
const SPI_BUFFER_BYTES: usize = 4096;

quick_error! {
    #[derive(Debug)]
/// Errors that can occur while using Blinkt.
    pub enum Error {
/// Accessing the GPIO peripheral returned an error.
///
/// Some of these errors can be fixed by changing file permissions, or upgrading
/// to a more recent version of Raspbian.
        Gpio(err: GpioError) { description(err.description()) from() }
/// Accessing the SPI peripheral returned an error.
        Spi(err: SpiError) { description(err.description()) from() }
/// An IO operation returned an error.
        Io(err: io::Error) { description(err.description()) from() }
    }
}

/// Result type returned from methods that can have `blinkt::Error`s.
pub type Result<T> = result::Result<T, Error>;

#[derive(Debug, Copy, Clone)]
struct Pixel {
    value: [u8; 4], // Brightness, blue, green, red
}

impl Pixel {
    fn set_rgb(&mut self, red: u8, green: u8, blue: u8) {
        self.value[1] = blue;
        self.value[2] = green;
        self.value[3] = red;
    }

    fn set_brightness(&mut self, brightness: f32) {
        self.value[0] = 0b1110_0000 | ((31.0 * brightness.max(0.0).min(1.0)) as u8);
    }

    fn set_rgbb(&mut self, red: u8, green: u8, blue: u8, brightness: f32) {
        self.set_rgb(red, green, blue);
        self.set_brightness(brightness);
    }

    fn bytes(&self) -> &[u8] {
        &self.value
    }
}

impl Default for Pixel {
    fn default() -> Pixel {
        Pixel {
            value: [0b1110_0000 | DEFAULT_BRIGHTNESS, 0, 0, 0],
        }
    }
}

trait SerialOutput {
    fn cleanup(&mut self);
    fn flush(&mut self) -> Result<()>;
    fn write(&mut self, data: &[u8]) -> Result<()>;
}

struct BlinktGpio {
    gpio: Gpio,
    pin_data: u8,
    pin_clock: u8,
}

impl BlinktGpio {
    pub fn with_settings(pin_data: u8, pin_clock: u8) -> Result<BlinktGpio> {
        let mut gpio = Gpio::new()?;

        gpio.set_mode(pin_data, Mode::Output);
        gpio.write(pin_data, Level::Low);
        gpio.set_mode(pin_clock, Mode::Output);
        gpio.write(pin_clock, Level::Low);

        Ok(BlinktGpio {
            gpio,
            pin_data,
            pin_clock,
        })
    }
}

impl SerialOutput for BlinktGpio {
    fn cleanup(&mut self) {
        self.gpio.cleanup();
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        for byte in data {
            for n in 0..8 {
                if (byte & (1 << (7 - n))) > 0 {
                    self.gpio.write(self.pin_data, Level::High);
                } else {
                    self.gpio.write(self.pin_data, Level::Low);
                }

                self.gpio.write(self.pin_clock, Level::High);
                self.gpio.write(self.pin_clock, Level::Low);
            }
        }

        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

struct BlinktSpi {
    spi: spi::Spi,
    buffer: [u8; SPI_BUFFER_BYTES],
    index: usize,
}

impl BlinktSpi {
    pub fn with_settings(clock_speed_hz: u32) -> Result<BlinktSpi> {
        Ok(BlinktSpi {
            spi: spi::Spi::new(
                spi::Bus::Spi0,
                spi::SlaveSelect::Ss0,
                clock_speed_hz,
                spi::Mode::Mode0,
            )?,
            buffer: [0; SPI_BUFFER_BYTES],
            index: 0,
        })
    }
}

impl SerialOutput for BlinktSpi {
    fn cleanup(&mut self) {}

    // Queues bytes for transmission. Data is sent only when 4096 bytes are buffered or flush() is called.
    fn write(&mut self, data: &[u8]) -> Result<()> {
        for val in data {
            self.buffer[self.index] = *val;
            self.index += 1;
            if self.index >= SPI_BUFFER_BYTES {
                self.flush()?;
            }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.spi.write(&self.buffer[0 .. self.index])?;
        self.index = 0;
        Ok(())
    }
}


/// Interface for the Pimoroni Blinkt!, and any similar APA102 or SK9822 LED
/// strips or boards.
///
/// By default, Blinkt is set up to communicate with an 8-pixel board through
/// data pin GPIO 23 (physical pin 16) and clock pin GPIO 24 (physical pin 18).
/// These settings can be changed to support alternate configurations.
pub struct Blinkt {
    serial_output: Box<dyn SerialOutput>,
    pixels: Vec<Pixel>,
    clear_on_drop: bool,
    end_frame: Vec<u8>,
}

impl Blinkt {
    /// Creates a new `Blinkt` using the default settings for a Pimoroni
    /// Blinkt! board.
    ///
    /// This sets the data pin to GPIO 23 (physical pin 16), the clock pin to
    /// GPIO 24 (physical pin 18), and number of pixels to 8.
    pub fn new() -> Result<Blinkt> {
        Blinkt::with_settings(DAT, CLK, NUM_PIXELS)
    }

    /// Creates a new `Blinkt` using bitbanging mode, with custom settings for
    /// the data pin, clock pin, and number of pixels. Pins should be specified
    /// by their BCM GPIO pin numbers.
    pub fn with_settings(pin_data: u8, pin_clock: u8, num_pixels: usize) -> Result<Blinkt> {
        Ok(Blinkt {
            serial_output: Box::new(BlinktGpio::with_settings(pin_data, pin_clock)?),
            pixels: vec![Pixel::default(); num_pixels],
            clear_on_drop: true,
            end_frame: vec![0u8; 4 + (((num_pixels as f32 / 16.0f32) + 0.94f32) as usize)],
        })
    }

    /// Creates a new `Blinkt` using hardware SPI, with custom settings for the
    /// clock speed and number of pixels.
    ///
    /// This sets the data pin to GPIO 10 (physical pin 19) and the clock pin
    /// to GPIO 11 (physical pin 23).
    ///
    /// The Raspberry Pi allows SPI clock speeds up to 125MHz (125_000_000),
    /// but the maximum speed supported by LED strips depends a lot on the
    /// number of pixels and wire quality, and requires some experimentation.
    /// 32MHz (32_000_000) seems to be the maximum clock speed for a typical
    /// short LED strip. Visit the [Raspberry Pi SPI Documentation](https://www.raspberrypi.org/documentation/hardware/raspberrypi/spi/)
    /// page for a complete list of supported clock speeds.
    pub fn with_spi(clock_speed_hz: u32, num_pixels: usize) -> Result<Blinkt> {
        Ok(Blinkt {
            serial_output: Box::new(BlinktSpi::with_settings(clock_speed_hz)?),
            pixels: vec![Pixel::default(); num_pixels],
            clear_on_drop: true,
            end_frame: vec![0u8; 4 + (((num_pixels as f32 / 16.0f32) + 0.94f32) as usize)],
        })
    }

    /// When enabled, clears all pixels when the `Blinkt` goes out of scope.
    ///
    /// Drop methods aren't called when a program is abnormally terminated,
    /// for instance when a user presses Ctrl-C, and the SIGINT signal isn't
    /// caught. You'll either have to catch those using crates such as
    /// `simple_signal`, or manually call `cleanup()`.
    ///
    /// Enabled by default.
    pub fn set_clear_on_drop(&mut self, clear_on_drop: bool) {
        self.clear_on_drop = clear_on_drop;
    }

    /// Changes the GPIO pin mode for the data and clock pins back to their
    /// original state, and optionally clears all pixels.
    ///
    /// Normally, this method is automatically called when Blinkt goes out of
    /// scope, but you can manually call it to handle early/abnormal termination.
    pub fn cleanup(&mut self) -> Result<()> {
        if self.clear_on_drop {
            self.clear();
            self.show()?;
        }

        self.serial_output.cleanup();

        Ok(())
    }

    /// Sets the red, green and blue values for a single pixel in the local
    /// buffer.
    ///
    /// For an 8-pixel board, valid values for pixel are 0-7. Valid values
    /// for red, green and blue are 0-255.
    pub fn set_pixel(&mut self, pixel: usize, red: u8, green: u8, blue: u8) {
        if let Some(pixel) = self.pixels.get_mut(pixel) {
            pixel.set_rgb(red, green, blue);
        }
    }

    /// Sets the red, green, blue and brightness values for a single pixel in
    /// the local buffer.
    ///
    /// For an 8-pixel board, valid values for pixel are 0-7. Valid
    /// values for red, green and blue are 0-255. Valid values for brightness
    /// are 0.0-1.0, which is converted to a 5-bit value.
    pub fn set_pixel_rgbb(&mut self, pixel: usize, red: u8, green: u8, blue: u8, brightness: f32) {
        if let Some(pixel) = self.pixels.get_mut(pixel) {
            pixel.set_rgbb(red, green, blue, brightness);
        }
    }

    /// Sets the brightness value for a single pixel in the local buffer.
    ///
    /// For an 8-pixel board, valid values for pixel are 0-7. Valid
    /// values for brightness are 0.0-1.0, which is converted to a
    /// 5-bit value.
    pub fn set_pixel_brightness(&mut self, pixel: usize, brightness: f32) {
        if let Some(pixel) = self.pixels.get_mut(pixel) {
            pixel.set_brightness(brightness);
        }
    }

    /// Sets the red, green and blue values for all pixels in the local buffer.
    ///
    /// Valid values for red, green and blue are 0-255.
    pub fn set_all_pixels(&mut self, red: u8, green: u8, blue: u8) {
        for pixel in &mut self.pixels {
            pixel.set_rgb(red, green, blue);
        }
    }

    /// Sets the red, green, blue and brightness values for all pixels in the
    /// local buffer.
    ///
    /// Valid values for red, green and blue are 0-255. Valid values for
    /// brightness are 0.0-1.0, which is converted to a 5-bit value.
    pub fn set_all_pixels_rgbb(&mut self, red: u8, green: u8, blue: u8, brightness: f32) {
        for pixel in &mut self.pixels {
            pixel.set_rgbb(red, green, blue, brightness);
        }
    }

    /// Sets the brightness value for all pixels in the local buffer.
    ///
    /// Valid values for brightness are 0.0-1.0, which is converted to a 5-bit
    /// value.
    pub fn set_all_pixels_brightness(&mut self, brightness: f32) {
        for pixel in &mut self.pixels {
            pixel.set_brightness(brightness);
        }
    }

    /// Sets the red, green and blue values to 0 for all pixels in the local
    /// buffer.
    pub fn clear(&mut self) {
        self.set_all_pixels(0, 0, 0);
    }

    /// Sends the contents of the local buffer to the pixels, updating their
    /// LED colors and brightness.
    pub fn show(&mut self) -> Result<()> {
        // Start frame (32*0).
        self.serial_output.write(&[0u8; 4])?;

        // LED frames (3*1, 5*brightness, 8*blue, 8*green, 8*red).
        for pixel in &self.pixels {
            self.serial_output.write(pixel.bytes())?;
        }

        // End frame (8*0 for every 16 pixels, 32*0 SK9822 reset frame).
        // The SK9822 won't update any pixels until it receives the next
        // start frame (32*0). The APA102 doesn't care if we send zeroes
        // instead of ones as the end frame. This workaround is
        // compatible with both the APA102 and SK9822.
        self.serial_output.write(&self.end_frame)?;
        self.serial_output.flush()?;
        Ok(())
    }
}

impl Drop for Blinkt {
    fn drop(&mut self) {
        self.cleanup().unwrap_or(());
    }
}

#[test]
fn test_new() {
    let mut blinkt = match Blinkt::new() {
        // Errors are acceptable, since they're likely caused by outside
        // distro/filesystem issues.
        Err(_) => return,
        Ok(blinkt) => blinkt,
    };

    blinkt.set_clear_on_drop(false);
}

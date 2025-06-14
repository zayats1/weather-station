use embassy_time::Delay;
// code from https://github.com/plorefice/dht11-rs
// I made it async
// I need to diable interrupts
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal_async::delay::DelayNs;

/// How long to wait for a pulse on the data line (in microseconds).

const TIMEOUT_US: u16 = 1000;

/// Error type for this crate.

#[derive(Debug, defmt::Format)]

pub enum Error<E> {
    /// Timeout during communication.
    Timeout,

    /// CRC mismatch.
    CrcMismatch,

    /// GPIO error.
    Gpio(E),
}

/// A DHT11 device.

pub struct Dht11<GPIO> {
    /// The concrete GPIO pin implementation.
    gpio: GPIO,
}

/// Results of a reading performed by the DHT11.

#[derive(Copy, Clone, Default, Debug, defmt::Format)]

pub struct Measurement {
    /// The measured temperature in tenths of degrees Celsius.
    pub temperature: f32,

    /// The measured humidity in tenths of a percent.
    pub humidity: f32,
}

impl<GPIO, E> Dht11<GPIO>
where
    GPIO: InputPin<Error = E> + OutputPin<Error = E>,
{
    /// Creates a new DHT11 device connected to the specified pin.

    pub fn new(gpio: GPIO) -> Self {
        Dht11 { gpio }
    }

    /// Destroys the driver, returning the GPIO instance.

    pub fn destroy(self) -> GPIO {
        self.gpio
    }

    /// Performs a reading of the sensor.

    pub async fn read(&mut self) -> Result<Measurement, Error<E>>
where {
        let mut delay = Delay;
        let mut data = [0u8; 5];

        // Perform initial handshake

        self.perform_handshake(&mut delay).await?;

        // Read bits

        for i in 0..40 {
            data[i / 8] <<= 1;

            if self.read_bit(&mut delay).await? {
                data[i / 8] |= 1;
            }
        }

        // Finally wait for line to go idle again.

        self.wait_for_pulse(true, &mut delay).await?;

        // Check CRC

        let crc = data[0]
            .wrapping_add(data[1])
            .wrapping_add(data[2])
            .wrapping_add(data[3]);

        // if crc != data[4] {
        //     return Err(Error::CrcMismatch);
        // }

        // Compute temperature

        let mut temp = i16::from(data[2] & 0x7f) * 10 + i16::from(data[3]);

        if data[2] & 0x80 != 0 {
            temp = -temp;
        }

        Ok(Measurement {
            temperature: temp as f32 / 10.0,

            humidity: (u16::from(data[0]) * 10 + u16::from(data[1])) as f32 / 10.0,
        })
    }

    pub async fn read_with_crc_check(&mut self) -> Result<Measurement, Error<E>>
where {
        let mut delay = Delay;
        let mut data = [0u8; 5];

        // Perform initial handshake

        self.perform_handshake(&mut delay).await?;

        // Read bits

        for i in 0..40 {
            data[i / 8] <<= 1;

            if self.read_bit(&mut delay).await? {
                data[i / 8] |= 1;
            }
        }

        // Finally wait for line to go idle again.

        self.wait_for_pulse(true, &mut delay).await?;

        // Check CRC

        let crc = data[0]
            .wrapping_add(data[1])
            .wrapping_add(data[2])
            .wrapping_add(data[3]);

        if crc != data[4] {
            return Err(Error::CrcMismatch);
        }

        // Compute temperature

        let mut temp = i16::from(data[2] & 0x7f) * 10 + i16::from(data[3]);

        if data[2] & 0x80 != 0 {
            temp = -temp;
        }

        Ok(Measurement {
            temperature: temp as f32 / 10.0,

            humidity: (u16::from(data[0]) * 10 + u16::from(data[1])) as f32 / 10.0,
        })
    }

    async fn perform_handshake<D>(&mut self, delay: &mut D) -> Result<(), Error<E>>
    where
        D: DelayNs,
    {
        // Set pin as floating to let pull-up raise the line and start the reading process.

        self.set_input()?;

        delay.delay_ms(1).await;

        // Pull line low for at least 18ms to send a start command.

        self.set_low()?;

        delay.delay_ms(25).await;

        // Restore floating

        self.set_input()?;

        delay.delay_us(40).await;

        // As a response, the device pulls the line low for 80us and then high for 80us.

        self.read_bit(delay).await?;

        Ok(())
    }

    async fn read_bit<D>(&mut self, delay: &mut D) -> Result<bool, Error<E>>
    where
        D: DelayNs,
    {
        let low = self.wait_for_pulse(true, delay).await?;

        let high = self.wait_for_pulse(false, delay).await?;

        Ok(high > low)
    }

    async fn wait_for_pulse<D>(&mut self, level: bool, delay: &mut D) -> Result<u32, Error<E>>
    where
        D: DelayNs,
    {
        let mut count = 0;

        while self.read_line()? != level {
            count += 1;

            if count > TIMEOUT_US {
                return Err(Error::Timeout);
            }

            delay.delay_us(1).await;
        }

        return Ok(u32::from(count));
    }

    fn set_input(&mut self) -> Result<(), Error<E>> {
        self.gpio.set_high().map_err(Error::Gpio)
    }

    fn set_low(&mut self) -> Result<(), Error<E>> {
        self.gpio.set_low().map_err(Error::Gpio)
    }

    fn read_line(&mut self) -> Result<bool, Error<E>> {
        self.gpio.is_high().map_err(Error::Gpio)
    }
}

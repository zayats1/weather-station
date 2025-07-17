#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
use defmt::{info, println};
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Flex, InputConfig, OutputConfig, Pull};
use weather_station::sensors::dht11::Dht11;
use {esp_backtrace as _, esp_println as _};

#[esp_hal_embassy::main]
async fn main(_spawner: Spawner) {
    // generator version: 0.3.1
    esp_bootloader_esp_idf::esp_app_desc!();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    use esp_hal::timer::systimer::SystemTimer;
    let systimer = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(systimer.alarm0);

    info!("Embassy initialized!");

    let mut dht11_pin = Flex::new(peripherals.GPIO4);
    dht11_pin.apply_output_config(
        &OutputConfig::default()
            .with_drive_mode(esp_hal::gpio::DriveMode::OpenDrain)
            .with_drive_strength(esp_hal::gpio::DriveStrength::_40mA)
            .with_pull(Pull::Up),
    );
    dht11_pin.apply_input_config(&InputConfig::default().with_pull(Pull::Up));
    dht11_pin.set_output_enable(true);
    dht11_pin.set_input_enable(true);

    let mut dht11 = Dht11::new(dht11_pin);
    let mut delay = Delay;
    loop {
        info!("Unchecked");
        let measurments = critical_section::with(|_| dht11.read(&mut delay)).await;
        println!("{:?}", measurments);
        Timer::after(Duration::from_millis(1000)).await; // >=1s interval between measturments is suitable
        info!("CRC");
        let measurments = critical_section::with(|_| dht11.read_with_crc_check(&mut delay)).await;
        println!("{:?}", measurments);
        Timer::after(Duration::from_millis(1000)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-beta.0/examples/src/bin
}

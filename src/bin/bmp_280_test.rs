#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
use bme280::i2c::AsyncBME280;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Timer};
use esp_hal::i2c::master::Config;
use esp_hal::{clock::CpuClock, i2c::master::I2c};
use esp_println::println;
use {esp_backtrace as _, esp_println as _};
use esp_hal::interrupt::software::SoftwareInterruptControl;

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    // generator version: 0.3.1
    esp_bootloader_esp_idf::esp_app_desc!();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0);

    let software_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);

    esp_rtos::start(timg0.timer0, software_interrupt.software_interrupt0);


  
    info!("Embassy initialized!");

    let i2c_bus = I2c::new(peripherals.I2C0, Config::default())
        .unwrap()
        .with_sda(peripherals.GPIO8)
    .with_scl(peripherals.GPIO9)
        .into_async();
    let mut bme280 = AsyncBME280::new_primary(i2c_bus);
    let mut delay = Delay;
    bme280.init(&mut delay).await.unwrap();
    loop {
        info!("Hello world!");
        let measurments = bme280.measure(&mut delay).await;
        println!("{:?}", measurments);
        Timer::after(Duration::from_secs(1)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-beta.0/examples/src/bin
}

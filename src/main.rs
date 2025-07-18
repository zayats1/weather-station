#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use bme280::i2c::AsyncBME280;

use esp_hal::time::Rate;

use core::{net::Ipv4Addr, str::FromStr};

use embassy_executor::Spawner;
use embassy_net::Ipv4Cidr;
use embassy_net::{StackResources, StaticConfigV4};
use embassy_time::{Delay, Duration, Timer};

use esp_hal::gpio::{Flex, InputConfig, OutputConfig, Pull};
use esp_hal::i2c;
use esp_hal::{clock::CpuClock, rng::Rng, timer::timg::TimerGroup};

use esp_wifi::{init, EspWifiController};
use num_traits::float::FloatCore;
use picoserve::{AppRouter, AppWithStateBuilder};

use esp_hal::i2c::master::I2c;
use weather_station::http_server::server::{web_task, AppProps, AppState};
use weather_station::network::dhcp::run_dhcp;
use weather_station::network::network_tasks::connection;
use weather_station::network::network_tasks::net_task;
use weather_station::sensors::dht11::Dht11;
use weather_station::{
    make_static, to_kpa, HumiditySender, NormalizedMeasurments, TheChannel, TheHumidityChannel,
};

use defmt::{debug, info, warn};

const GW_IP_ADDR_ENV: Option<&'static str> = Some("192.168.1.1");
const SSID: &str = "WeatherStation";

const HUMIDITY_MEASURMENT_INTERVAL: Duration = Duration::from_millis(1250);
const INTERVAL: Duration = Duration::from_millis(100);
type Dht = Dht11<Flex<'static>>;

use panic_rtt_target as _;
// use esp_alloc as _;
// use esp_backtrace as _;
// use esp_println as _;

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    rtt_target::rtt_init_defmt!();
    esp_bootloader_esp_idf::esp_app_desc!();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 75 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);

    use esp_hal::timer::systimer::SystemTimer;
    let systimer = SystemTimer::new(peripherals.SYSTIMER);
    esp_hal_embassy::init(systimer.alarm0);

    let mut delay = Delay;
    // I2C0 conflicts with wifi in esp32

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

    let dht11 = Dht11::new(dht11_pin);

    let esp_wifi_ctrl =
        &*make_static!(EspWifiController<'static>, init(timg0.timer0, rng).unwrap());

    let (controller, interfaces) = esp_wifi::wifi::new(esp_wifi_ctrl, peripherals.WIFI).unwrap();

    let device = interfaces.ap;

    let i2c0 = I2c::new(
        peripherals.I2C0,
        i2c::master::Config::default().with_frequency(Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(peripherals.GPIO8)
    .with_scl(peripherals.GPIO9)
    .into_async();
    // I use BMP280, it is similar, except humidity

    let mut bme280 = AsyncBME280::new_primary(i2c0);
    bme280.init(&mut delay).await.unwrap();

    let gw_ip_addr_str = GW_IP_ADDR_ENV.unwrap_or("192.168.2.1");
    let gw_ip_addr = Ipv4Addr::from_str(gw_ip_addr_str).expect("failed to parse gateway ip");

    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr, 24),
        gateway: Some(gw_ip_addr),
        dns_servers: Default::default(),
    });

    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        device,
        config,
        make_static!(StackResources<8>, StackResources::<8>::new()),
        seed,
    );

    spawner.spawn(connection(controller, SSID)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(run_dhcp(stack, gw_ip_addr_str)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("DHCP is enabled so there's no need to configure a static IP, just in case:");
    while !stack.is_config_up() {
        Timer::after(Duration::from_millis(100)).await
    }
    stack
        .config_v4()
        .inspect(|c| debug!("ipv4 config: {:?}", c));

    let channel = make_static!(TheChannel, TheChannel::new());
    let server_receiver = channel.receiver();
    let data_sender = channel.sender();

    let humidity_channel = make_static!(TheHumidityChannel, TheHumidityChannel::new());
    let humidity_receiver = humidity_channel.receiver();
    let humidity_sender = humidity_channel.sender();

    let app = make_static!(AppRouter<AppProps>, AppProps.build_app());

    let config = make_static!(
        picoserve::Config::<Duration>,
        picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Some(Duration::from_secs(5)),
            persistent_start_read_request: Some(Duration::from_secs(1)),
            read_request: Some(Duration::from_secs(1)),
            write: Some(Duration::from_secs(1)),
        })
        .keep_connection_alive()
    );

    spawner.must_spawn(web_task(stack, app, config, AppState::new(server_receiver)));
    spawner.must_spawn(measure_humidity(dht11, humidity_sender));
    let mut humidity = 0.0f32;
    loop {
        info!("Measurments");

        let measurments = bme280.measure(&mut delay).await;

        if let Ok(received_humidity) = humidity_receiver.try_receive() {
            if received_humidity <= 100.0 {
                humidity = round_up(received_humidity);
            }
        }
        // Todo error handling
        if let Ok(measurments) = measurments {
            let normalized = NormalizedMeasurments {
                pressure: round_up(to_kpa(measurments.pressure)),
                humidity,
                temperature: round_up(measurments.temperature),
            };

            data_sender.send(normalized).await;
        }
        Timer::after(INTERVAL).await;
    }
}

fn round_up(val: f32) -> f32 {
    let shifted = val * 10.0;
    shifted.round() / 10.0
}

#[embassy_executor::task]
async fn measure_humidity(mut dht11: Dht, sender: HumiditySender) {
    let mut delay = Delay;
    loop {
        let humidity_and_temp = critical_section::with(|_| dht11.read(&mut delay)).await;
        Timer::after(HUMIDITY_MEASURMENT_INTERVAL).await;
        match humidity_and_temp {
            Ok(humidity_and_temp) => sender.send(humidity_and_temp.humidity).await,
            Err(e) => warn!("{:?}", e),
        }
    }
}

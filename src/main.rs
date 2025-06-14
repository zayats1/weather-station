#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]
use core::{net::Ipv4Addr, str::FromStr};

use bme280::i2c::AsyncBME280;
use embassy_executor::Spawner;
use embassy_net::Ipv4Cidr;
use embassy_net::{StackResources, StaticConfigV4};

use embassy_time::{Delay, Duration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::i2c::master::{Config, I2c};
use esp_hal::{clock::CpuClock, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_wifi::{EspWifiController, init};
use picoserve::{AppRouter, AppWithStateBuilder};
use weather_station::http_server::server::{web_task, AppProps, AppState};
use weather_station::make_static;
use weather_station::network::dhcp::run_dhcp;
use weather_station::network::network_tasks::connection;
use weather_station::network::network_tasks::net_task;
use weather_station::TheChannel;
const GW_IP_ADDR_ENV: Option<&'static str> = Some("192.168.1.1");
const SSID: &'static str = "weather-station";
#[esp_hal_embassy::main]

async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 57 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let mut rng = Rng::new(peripherals.RNG);

    let esp_wifi_ctrl = &*make_static!(
        EspWifiController<'static>,
        init(timg0.timer0, rng.clone(), peripherals.RADIO_CLK).unwrap()
    );

    let (controller, interfaces) = esp_wifi::wifi::new(&esp_wifi_ctrl, peripherals.WIFI).unwrap();

    let device = interfaces.ap;

    cfg_if::cfg_if! {
        if #[cfg(feature = "esp32")] {
            let timg1 = TimerGroup::new(peripherals.TIMG1);
            esp_hal_embassy::init(timg1.timer0);
        } else {
            use esp_hal::timer::systimer::SystemTimer;
            let systimer = SystemTimer::new(peripherals.SYSTIMER);
            esp_hal_embassy::init(systimer.alarm0);
        }
    }
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
        make_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    let i2c_bus = I2c::new(peripherals.I2C0, Config::default())
        .unwrap()
        .with_scl(peripherals.GPIO25).with_sda(peripherals.GPIO26).into_async();
    let mut bme280 = AsyncBME280::new_primary(i2c_bus);
    let mut delay = Delay;
    bme280.init(&mut delay).await.unwrap();

    spawner.spawn(connection(controller, SSID)).ok();
    spawner.spawn(net_task(runner)).ok();
    spawner.spawn(run_dhcp(stack, gw_ip_addr_str)).ok();


    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("DHCP is enabled so there's no need to configure a static IP, just in case:");
    while !stack.is_config_up() {
        Timer::after(Duration::from_millis(100)).await
    }
    stack
        .config_v4()
        .inspect(|c| println!("ipv4 config: {c:?}"));

    // a tassk for the  web server
    let channel   =make_static!(TheChannel,TheChannel::new());
    let server_receiver = channel.receiver();
    let server_sender  = channel.sender();


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

    spawner.must_spawn(web_task(stack, app, config,AppState::new(server_receiver)));
}




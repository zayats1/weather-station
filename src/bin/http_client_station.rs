#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use core::fmt::Write;

use bme280::i2c::AsyncBME280;

use embassy_net::tcp::client::{TcpClient, TcpClientState};
use esp_hal::time::Rate;
use esp_radio::wifi::WifiController;
use heapless::String;
use reqwless::client::HttpClient;

use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::{Delay, Duration, Timer};

use esp_hal::gpio::{Flex, InputConfig, OutputConfig, Pull};
use esp_hal::i2c;
use esp_hal::{clock::CpuClock, rng::Rng, timer::timg::TimerGroup};

use num_traits::float::FloatCore;

use defmt::{debug, error, info, println};
use embassy_net::dns::DnsSocket;
use esp_hal::i2c::master::I2c;

use reqwless::request::Method;

use weather_station::network::network_tasks::net_task;
use weather_station::sensors::dht11::Dht11;
use weather_station::{
    HumiditySender, NormalizedMeasurments, ServerReceiver, TheChannel, TheHumidityChannel,
    make_static, to_kpa,
};

const HUMIDITY_MEASURMENT_INTERVAL: Duration = Duration::from_millis(1250);
const INTERVAL: Duration = Duration::from_millis(100);
type Dht = Dht11<Flex<'static>>;

use esp_alloc as _;
use panic_rtt_target as _;
// use esp_backtrace as _;
// use esp_println as _;

// export ssid = SSID
const SSID: &str = match option_env!("SSID") {
    Some(ssid) => ssid,
    None => "",
};

const PASSWORD: &str = match option_env!("PASSWORD") {
    Some(pass) => pass,
    None => "",
};

const WEBSITE: &str = match option_env!("WEBSITE") {
    Some(pass) => pass,
    None => "",
};

const REQUEST_INTERVAL: Option<&str> = option_env!("REQUEST_INTERVAL");

const DEFAULT_REQUEST_INTERVAL: Duration = Duration::from_millis(2000);

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    rtt_target::rtt_init_defmt!();
    esp_bootloader_esp_idf::esp_app_desc!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36
         * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);

    // THIS IS IMPORTANT FOR WIFI AND BLE: You MUST start the scheduler
    // before initializing the radio!
    info!("Begin0");
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    info!("BeginWifi");

    let station_config = esp_radio::wifi::Config::Station(
        esp_radio::wifi::sta::StationConfig::default()
            .with_ssid(SSID.try_into().unwrap())
            .with_authentication(esp_radio::wifi::AuthenticationMethodConfig::Wpa2Personal(
                PASSWORD.try_into().unwrap(),
            )),
    );
    info!("Starting wifi");

    let wifi_sta_interface = esp_radio::wifi::Interface::station();
    let controller = esp_radio::wifi::WifiController::new(
        peripherals.WIFI,
        esp_radio::wifi::ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();
    info!("Wifi started!");

    let config = embassy_net::Config::dhcpv4(Default::default());

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        wifi_sta_interface,
        config,
        make_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

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

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());
    //spawner.spawn(weather_station::network::dhcp::run_dhcp(stack, GW_IP_ADDR_ENV).unwrap());

    stack
        .config_v4()
        .inspect(|c| debug!("ipv4 config: {:?}", c));

    let channel = make_static!(TheChannel, TheChannel::new());
    let server_receiver = channel.receiver();
    let data_sender = channel.sender();

    let humidity_channel = make_static!(TheHumidityChannel, TheHumidityChannel::new());
    let humidity_receiver = humidity_channel.receiver();
    let humidity_sender = humidity_channel.sender();

    spawner.spawn(web_task(stack, server_receiver).unwrap());
    spawner.spawn(measure_humidity(dht11, humidity_sender).unwrap());

    let mut humidity = 0.0f32;
    loop {
        info!("Measurments");

        let measurments = bme280.measure(&mut delay).await;

        if let Ok(received_humidity) = humidity_receiver.try_receive()
            && received_humidity <= 100.0
        {
            humidity = round_up(received_humidity);
        }
        // Todo error handling

        if let Ok(measurments) = measurments {
            // debug!("measurments {:?}",measurments);
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
        debug!("humidity_and_temp: {}", humidity_and_temp);
        match humidity_and_temp {
            Ok(humidity_and_temp) => sender.send(humidity_and_temp.humidity).await,

            Err(e) => error!("humidity_and_temp: {:?}", e),
        }
    }
}

#[embassy_executor::task]
pub async fn web_task(stack: embassy_net::Stack<'static>, receiver: ServerReceiver) {
    let dns_client = DnsSocket::new(stack);

    let tcp_client = TcpClient::new(
        stack,
        make_static!(
            TcpClientState<1, 1500, 1500>,
            TcpClientState::<1, 1500, 1500>::new()
        ),
    );

    let mut client = HttpClient::new(&tcp_client, &dns_client);
    let mut rx_buffer = [0u8; 1024];
    let request_interval = REQUEST_INTERVAL
        .map(|t| t.parse::<u64>().ok().map(|t| Duration::from_millis(t)))
        .flatten()
        .unwrap_or(DEFAULT_REQUEST_INTERVAL);

    loop {
        let measturments = receiver.receive().await;
        let temperature = measturments.temperature;
        defmt::println!("{:?}", measturments);
        let mut url = String::<200>::new();
        url.clear();
        write!(&mut url, "{}/update/1/{}", WEBSITE, temperature,).unwrap();

        let sending_result = client.request(Method::POST, url.as_str()).await;
        match sending_result {
            Ok(mut reqwest) => {
                println!("HTTP request successful");
                match reqwest.send(&mut rx_buffer).await {
                    Ok(_) => println!("sended"),
                    Err(_) => (),
                }
            }
            Err(e) => {
                println!("try again {}", e);
            }
        }

        Timer::after(request_interval).await;
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");

    loop {
        println!("About to connect...");

        match controller.connect_async().await {
            Ok(info) => {
                println!("Wifi connected to {:?}", info);

                // wait until we're no longer connected
                let info = controller.wait_for_disconnect_async().await.ok();
                println!("Disconnected: {:?}", info);
            }
            Err(e) => {
                println!("Failed to connect to wifi: {:?}", e);
            }
        }

        Timer::after(Duration::from_millis(5000)).await
    }
}

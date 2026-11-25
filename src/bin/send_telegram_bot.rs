#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use core::fmt::Write;

use bme280::i2c::AsyncBME280;

use embassy_net::tcp::client::{TcpClient, TcpClientState};
use esp_hal::rng::{Trng, TrngSource};
use esp_hal::time::Rate;
use esp_hal::timer::OneShotTimer;
use heapless::String;
use reqwless::client::{HttpClient, TlsConfig};

use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_time::{Delay, Duration, Timer};

use esp_hal::gpio::{Flex, InputConfig, OutputConfig, Pull};
use esp_hal::i2c;
use esp_hal::{clock::CpuClock, rng::Rng, timer::timg::TimerGroup};

use defmt::{debug, error, info, println};
use embassy_net::dns::DnsSocket;
use esp_hal::i2c::master::I2c;

use reqwless::headers::ContentType;
use reqwless::request::{Method, RequestBuilder};

use weather_station::network::network_tasks::{connection_sta, net_task};
use weather_station::sensors::dht11::Dht11;
use weather_station::{
    NormalizedMeasurments, ServerReceiver, TheChannel, TheHumidityChannel, make_static, to_kpa,
};

use weather_station::round_up;

const INTERVAL: Duration = Duration::from_millis(200);

const REQUEST_INTERVAL: Option<&str> = option_env!("REQUEST_INTERVAL");

const DEFAULT_REQUEST_INTERVAL: Duration = Duration::from_millis(5000);

static CERT: &[u8] = include_bytes!("certs/telegram_ca.der");

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

const CHAT_ID: &str = match option_env!("CHAT_ID") {
    // http
    Some(pass) => pass,
    None => "",
};

const BOT_TOKEN: &str = match option_env!("BOT_TOKEN") {
    // http
    Some(pass) => pass,
    None => "",
};

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    rtt_target::rtt_init_defmt!();
    esp_bootloader_esp_idf::esp_app_desc!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 96 * 1024); // more ram for TLS

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

    spawner.spawn(connection_sta(controller).unwrap());
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

    let _trng_source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
    let trng = Trng::try_new().unwrap();
    let trng: &mut Trng = make_static!(Trng, trng);

    spawner.spawn(web_task(stack, server_receiver, trng).unwrap());

    let timg1 = TimerGroup::new(peripherals.TIMG1);
    let one_shot = OneShotTimer::new(timg1.timer0).into_async(); // dedicated driver is more stable
    spawner.spawn(
        weather_station::sensors::measure_humidity::measure_humidity(
            dht11,
            one_shot,
            humidity_sender,
        )
        .unwrap(),
    );

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

#[embassy_executor::task]
pub async fn web_task(
    stack: embassy_net::Stack<'static>,
    receiver: ServerReceiver,
    rng: &'static mut Trng,
) {
    let dns_client = DnsSocket::new(stack);

    let tcp_client = TcpClient::new(
        stack,
        make_static!(
            TcpClientState<1, 1500, 1500>,
            TcpClientState::<1, 1500, 1500>::new()
        ),
    );

    // Configure TLS (using TlsVerify::None for initial testing, or TlsVerify::Certificate)

    let mbedtls_lib_instance = mbedtls_rs::Tls::new(rng).unwrap();
    // For debug messages:
    // mbedtls_lib_instance.set_debug(1);

    let tls_config = TlsConfig::new(
        reqwless::TlsVersion::Tls1_2,
        reqwless::Certificate::new(reqwless::X509::DER(CERT)).unwrap(),
        None, // optionally, add client certificate for mTLS
        mbedtls_lib_instance.reference(),
    );

    // 3. Create the HTTPS client and send the request
    let mut client = HttpClient::new_with_tls(&tcp_client, &dns_client, tls_config);

    let mut rx_buffer = [0u8; 1024];
    let request_interval = REQUEST_INTERVAL
        .and_then(|t| t.parse::<u64>().ok().map(Duration::from_millis))
        .unwrap_or(DEFAULT_REQUEST_INTERVAL);
    let mut url = String::<200>::new();
    write!(url, "https://api.telegram.org/bot{}/sendMessage", BOT_TOKEN).unwrap(); // todo handling errors
    loop {
        let measturments = receiver.receive().await;
        let temperature = measturments.temperature;
        let humidity = measturments.humidity;
        let pressure = measturments.pressure;
        defmt::println!("{:?}", measturments);

        update_value(
            &mut client,
            &mut rx_buffer,
            &mut url,
            temperature,
            "Temperature",
            "*C",
        )
        .await;
        update_value(
            &mut client,
            &mut rx_buffer,
            &mut url,
            humidity,
            "Humidity",
            "%",
        )
        .await;
        update_value(
            &mut client,
            &mut rx_buffer,
            &mut url,
            pressure,
            "Atmospheric pressure",
            "kPa",
        )
        .await;

        Timer::after(request_interval).await;
    }
}

async fn update_value<'a, T, D, const N: usize>(
    client: &mut HttpClient<'a, T, D>,
    rx_buffer: &mut [u8],
    url: &mut String<N>,
    value: f32,
    label: &str,
    unit: &str,
) where
    T: embedded_nal_async::TcpConnect + 'a,
    D: embedded_nal_async::Dns + 'a,
{
    let mut payload = String::<256>::new();
    write!(
        payload,
        r#"{{"chat_id":"{}","text":"{}: {:.2} {}"}}"#,
        CHAT_ID, label, value, unit
    )
    .unwrap();

    let sending_result = client.request(Method::POST, url).await;
    match sending_result {
        Ok(reqwest) => {
            let reqwest = reqwest.content_type(ContentType::ApplicationJson);
            let mut reqwest = reqwest.body(payload.as_str().as_bytes());
            println!("HTTP request successful");
            if let Ok(response) = reqwest.send(rx_buffer).await {
                println!("sended");
                println!("Response:");
                let body = response.body().body_buf;
                let text = str::from_utf8(body).unwrap_or("No body");
                println!("{}", text);
            }
        }
        Err(e) => {
            error!("try again {}", e);
        }
    }
}

use defmt::{debug, info};
use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_wifi::wifi::{
    AccessPointConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiState,
};

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>, ssid: &'static str) {
    info!("start connection task");
    debug!("Device capabilities: {:?}", controller.capabilities());
    loop {
        if esp_wifi::wifi::wifi_state() == WifiState::ApStarted {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::AccessPoint(AccessPointConfiguration {
                ssid: ssid.into(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            info!("Starting wifi");
            controller.start_async().await.unwrap();
            info!("Wifi started!");
        }
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

use defmt::{debug, info};
use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{
    AccessPointConfig,  ModeConfig, WifiController, WifiDevice, WifiEvent, WifiApState
};

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>, ssid: &'static str) {
    info!("start connection task");
    debug!("Device capabilities: {:?}", controller.capabilities());
    loop {
        if esp_radio::wifi::ap_state() == WifiApState::Started {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::ApStop).await;
            Timer::after(Duration::from_millis(5000)).await
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::AccessPoint(AccessPointConfig::default().with_ssid(ssid.into()));
            controller.set_config(&client_config).unwrap();
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

use defmt::{debug, info};
use embassy_net::Runner;
use embassy_time::{Duration, Timer};
use esp_radio::wifi::{Interface, WifiController};

#[embassy_executor::task]
pub async fn connection(controller: WifiController<'static>) {
    info!("start connection task");
    loop {
        let ev = controller
            .wait_for_access_point_connected_event_async()
            .await;
        match ev {
            Ok(esp_radio::wifi::ap::EventInfo::Connected(info)) => {
                debug!("Station connected: {:?}", info);
            }
            Ok(esp_radio::wifi::ap::EventInfo::Disconnected(info)) => {
                debug!("Station disconnected: {:?}", info);
            }
            _ => (),
        }
        Timer::after(Duration::from_millis(5000)).await
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

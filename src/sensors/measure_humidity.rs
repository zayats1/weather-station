use crate::{HUMIDITY_MEASURMENT_INTERVAL, HumiditySender};
use defmt::{debug, error};
use esp_hal::{gpio::Flex, timer::OneShotTimer};

use super::dht11::Dht11;

type Dht = Dht11<Flex<'static>>;
#[embassy_executor::task]
pub async fn measure_humidity(
    mut dht11: Dht,
    mut timer: OneShotTimer<'static, esp_hal::Async>,
    sender: HumiditySender,
) {
    loop {
        let humidity_and_temp = critical_section::with(|_| dht11.read(&mut timer)).await;

        match humidity_and_temp {
            Ok(humidity_and_temp) => {
                debug!("humidity_and_temp: {}", humidity_and_temp);
                sender.send(humidity_and_temp.humidity).await;
            }

            Err(e) => error!("humidity_and_temp: {:?}", e),
        }
        timer
            .delay_millis_async(HUMIDITY_MEASURMENT_INTERVAL.as_millis() as u32)
            .await;
    }
}

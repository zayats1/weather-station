#![no_std]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]

use embassy_sync::{
    blocking_mutex::raw::NoopRawMutex,
    channel::{Channel, Receiver, Sender},
};
use serde::Serialize;

pub mod http_server;
pub mod network;
pub mod sensors;
/*

The macro makes a some object to have a static lifetime
which means it will live as long as the program
*/
#[macro_export]
macro_rules! make_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}
pub const MESSAGES: usize = 1;

#[derive(Debug, Serialize, defmt::Format)]
pub struct NormalizedMeasurments {
    pub pressure: f32,
    pub humidity: f32,
    pub temperature: f32,
}

pub type ServerReceiver = Receiver<'static, NoopRawMutex, NormalizedMeasurments, MESSAGES>;
pub type DataSender = Sender<'static, NoopRawMutex, NormalizedMeasurments, MESSAGES>;
pub type TheChannel = Channel<NoopRawMutex, NormalizedMeasurments, MESSAGES>;

pub type HumidityReceiver = Receiver<'static, NoopRawMutex, f32, MESSAGES>;
pub type HumiditySender = Sender<'static, NoopRawMutex, f32, MESSAGES>;
pub type TheHumidityChannel = Channel<NoopRawMutex, f32, MESSAGES>;

pub fn to_kpa(pressure: f32) -> f32 {
    pressure / 1000.0
}

#![no_std]
#![feature(type_alias_impl_trait)]
#![feature(impl_trait_in_assoc_type)]

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::{Receiver, Sender}};
use serde::Serialize;

pub mod network;
pub mod http_server;
pub mod sensors;
/*

The macro makes a some object to have a static lifetime
which means it will live as long as the program
*/
#[macro_export]
macro_rules! make_static {
    ($t:ty, $val:expr) => ($crate::make_static!($t, $val,));
    ($t:ty, $val:expr, $(#[$m:meta])*) => {{
        $(#[$m])*
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        STATIC_CELL.init_with(|| $val)
    }};
}

const MESSAGES:usize = 1;

#[derive(Debug,Serialize)]
pub struct NormalizedMeasurments{
    pressure:f32,
    humidiity:f32,
    temperature:f32
}
type ServerReceiver = Receiver<'static, CriticalSectionRawMutex,NormalizedMeasurments,MESSAGES>;
type DataSender = Sender<'static, CriticalSectionRawMutex,NormalizedMeasurments,MESSAGES>;
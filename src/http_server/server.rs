use core::str::FromStr;

use embassy_time::Duration;
use heapless::String;
use picoserve::extract::State;
use picoserve::routing::get;
use picoserve::AppRouter;
use picoserve::AppWithStateBuilder;
use crate::NormalizedMeasurments;
use crate::ServerReceiver;



pub struct AppState{
    receiver:ServerReceiver
}

impl AppState{
    pub fn new(receiver:ServerReceiver) -> Self{
        return Self{
            receiver
        };
    }
}

pub struct AppProps;


impl picoserve::extract::FromRef<AppState> for  ServerReceiver{
    fn from_ref(state: &AppState) -> Self {
         state.receiver
    }
}


impl  AppWithStateBuilder for AppProps {
    type State = AppState;
    type PathRouter = impl picoserve::routing::PathRouter<AppState>;

    fn build_app(self) ->  picoserve::Router<Self::PathRouter, Self::State> {
        picoserve::Router::new().route("/", 
        get(move|State(receiver) : State<ServerReceiver>| async move { 
           let measturments  = receiver.receive().await;
             serde_json_core::to_string::<NormalizedMeasurments,32>(&measturments).unwrap_or(String::from_str("No data").unwrap())
        }))
    }
}



#[embassy_executor::task]
pub async fn web_task(
    stack: embassy_net::Stack<'static>,
    app: &'static AppRouter<AppProps>,
    config: &'static picoserve::Config<Duration>,
    state: AppState,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::listen_and_serve_with_state(
        1,
        app,
        config,
        stack,
        port,
        &mut tcp_rx_buffer,
        &mut tcp_tx_buffer,
        &mut http_buffer,
        &state,
    )
    .await
}
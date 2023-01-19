use crate::pipeline::{populate_gdp_struct_from_bytes, proc_gdp_packet};
use crate::structs::{GDPChannel, GDPPacket, Packet};
use std::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Sender, Receiver};
use std::{net::SocketAddr, pin::Pin, str::FromStr};
use crate::structs::GDPName;
use crate::pipeline::construct_gdp_advertisement_from_bytes;
use crate::structs::get_gdp_name_from_topic;
const UDP_BUFFER_SIZE: usize = 4096; // 17480 17kb TODO: make it formal

/// handle one single session of tcpstream
/// 1. init and advertise the mpsc channel to connection rib
/// 2. select between
///         incoming tcp packets -> receive and send to rib
///         incomine packets from rib -> send to the tcp session
async fn handle_tcp_stream(
    stream: TcpStream, rib_tx: &Sender<GDPPacket>, channel_tx: &Sender<GDPChannel>,
    m_tx:Sender<GDPPacket>, mut m_rx:Receiver<GDPPacket>
) {
    // ...
    
    // TODO: placeholder, later replace with packet parsing

    loop {
        // Wait for the TCP socket to be readable
        // or new data to be sent
        tokio::select! {
            // new stuff from TCP!
            _f = stream.readable() => {
                // Creating the buffer **after** the `await` prevents it from
                // being stored in the async task.

                let mut buf = vec![0u8; UDP_BUFFER_SIZE];
                // Try to read data, this may still fail with `WouldBlock`
                // if the readiness event is a false positive.
                match stream.try_read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        println!("read {} bytes", n);
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(_e) => {
                        continue;
                    }
                }
                let packet = populate_gdp_struct_from_bytes(buf);
                proc_gdp_packet(packet,  // packet
                    rib_tx,  //used to send packet to rib
                    channel_tx, // used to send GDPChannel to rib
                    &m_tx //the sending handle of this connection
                ).await;

            },

            // new data to send to TCP!
            Some(pkt_to_forward) = m_rx.recv() => {
                // okay this may have deadlock
                stream.writable().await.expect("TCP stream is closed");

                info!("TCP packet to forward: {:?}", pkt_to_forward);
                let payload = pkt_to_forward.get_byte_payload().unwrap();
                // Try to write data, this may still fail with `WouldBlock`
                // if the readiness event is a false positive.
                match stream.try_write(&payload) {
                    Ok(n) => {
                        println!("write {} bytes", n);
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue
                    }
                    Err(_e) => {
                        println!("Err of other kind");
                        continue
                    }
                }
            },
        }
    }
}

/// listen at @param address and process on tcp accept()
///     rib_tx: channel that send GDPPacket to rib
///     channel_tx: channel that advertise GDPChannel to rib
pub async fn tcp_listener(addr: String, rib_tx: Sender<GDPPacket>, channel_tx: Sender<GDPChannel>) {
    let listener = TcpListener::bind(&addr).await.unwrap();
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let rib_tx = rib_tx.clone();
        let channel_tx = channel_tx.clone();

        // Process each socket concurrently.
        tokio::spawn(async move { 
            let (m_tx, mut m_rx) = mpsc::channel(32);
            handle_tcp_stream(socket, &rib_tx, &channel_tx, m_tx, m_rx).await 
        });
    }
}



pub async fn tcp_to_peer(addr: String, 
    rib_tx: Sender<GDPPacket>,
    channel_tx: Sender<GDPChannel>) {
    
    let stream = TcpStream::connect(SocketAddr::from_str(&addr).unwrap()).await.unwrap();
    println!("{:?}", stream);

    let m_gdp_name = GDPName(get_gdp_name_from_topic(addr.as_str())); 
    info!("TCP takes gdp name {:?}", m_gdp_name);

    let (m_tx, mut m_rx) = mpsc::channel(32);
    let node_advertisement = construct_gdp_advertisement_from_bytes(
        m_gdp_name, m_gdp_name
    );
    proc_gdp_packet(
        node_advertisement, // packet
        &rib_tx,            //used to send packet to rib
        &channel_tx,        // used to send GDPChannel to rib
        &m_tx,              //the sending handle of this connection
    )
    .await;
    handle_tcp_stream(stream, &rib_tx, &channel_tx, m_tx,m_rx).await;
}
use crate::pipeline::{
    construct_gdp_advertisement_from_bytes, construct_gdp_forward_from_bytes,
    populate_gdp_struct_from_bytes, proc_gdp_packet,
};
use crate::structs::{GDPChannel, GDPName, GDPPacket, GdpAction, Packet};
use futures::executor::LocalPool;
use futures::future;
use futures::stream::StreamExt;
use futures::task::LocalSpawnExt;
use pnet::packet;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json;
use std::str;
use tokio::sync::mpsc::{self, Receiver, Sender};
use sha2::{Sha256, Sha512, Digest};
#[cfg(feature = "ros")]
use r2r::QosProfile;
use std::mem::transmute;
use crate::structs::get_gdp_name_from_topic;

#[cfg(feature = "ros")]
pub async fn ros_listener(rib_tx: Sender<GDPPacket>, channel_tx: Sender<GDPChannel>)  {

    let publisher_name = "/GDP_Router";
    let publisher_gdp_name = GDPName(get_gdp_name_from_topic(publisher_name));
    info!("ROS takes gdp name {:?}", publisher_gdp_name);

    let topic_name = "/chatter";
    let topic_gdp_name = GDPName(get_gdp_name_from_topic(topic_name));
    info!("topic /chatter takes gdp name {:?}", topic_gdp_name);

    let (m_tx, mut m_rx) = mpsc::channel::<GDPPacket>(32);
    let ctx = r2r::Context::create().expect("context creation failure");
    let mut node =
        r2r::Node::create(ctx, "GDP_Router", "namespace").expect("node creation failure");
    let mut subscriber = node
        .subscribe_untyped(topic_name, "std_msgs/msg/String", QosProfile::default())
        .expect("topic subscribing failure");
    let publisher = node
        .create_publisher_untyped("/chatter_echo", "std_msgs/msg/String", QosProfile::default())
        .expect("publisher creation failure");

    let handle = tokio::task::spawn_blocking(move || loop {
        node.spin_once(std::time::Duration::from_millis(100));
    });

    // note that different from other connection ribs, we send advertisement ahead of time
    let node_advertisement = construct_gdp_advertisement_from_bytes(topic_gdp_name, publisher_gdp_name);
    proc_gdp_packet(
        node_advertisement, // packet
        &rib_tx,            //used to send packet to rib
        &channel_tx,        // used to send GDPChannel to rib
        &m_tx,              //the sending handle of this connection
    )
    .await;

    loop {
        tokio::select! {
            Some(packet) = subscriber.next() => {
                info!("received a packet {:?}", packet);
                let ros_msg = serde_json::to_vec(&packet.unwrap()).unwrap();

                let packet = construct_gdp_forward_from_bytes(topic_gdp_name,publisher_gdp_name, ros_msg );
                proc_gdp_packet(packet,  // packet
                    &rib_tx,  //used to send packet to rib
                    &channel_tx, // used to send GDPChannel to rib
                    &m_tx //the sending handle of this connection
                ).await;

            }
            Some(pkt_to_forward) = m_rx.recv() => {
                // okay this may have deadlock
                
                if (pkt_to_forward.action == GdpAction::Forward) {
                    info!("the crashing payload is {:#?}", pkt_to_forward);
                    let payload = pkt_to_forward.get_byte_payload().unwrap();
                    let ros_msg = serde_json::from_str(str::from_utf8(payload).unwrap()).expect("json parsing failure");
                    publisher.publish(ros_msg).unwrap();
                }
            },
        }
    }

    // handle.await;
}

#[cfg(feature = "ros")]
pub fn ros_sample() -> Result<(), Box<dyn std::error::Error>> {
    let ctx = r2r::Context::create()?;
    let mut node = r2r::Node::create(ctx, "node", "namespace")?;
    let subscriber =
        node.subscribe::<r2r::std_msgs::msg::String>("/topic", QosProfile::default())?;
    let publisher =
        node.create_publisher::<r2r::std_msgs::msg::String>("/topic", QosProfile::default())?;
    let mut timer = node.create_wall_timer(std::time::Duration::from_millis(1000))?;

    // Set up a simple task executor.
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();

    // Run the subscriber in one task, printing the messages
    spawner.spawn_local(async move {
        subscriber
            .for_each(|msg| {
                println!("got new msg: {}", msg.data);
                future::ready(())
            })
            .await
    })?;

    // Run the publisher in another task
    spawner.spawn_local(async move {
        let mut counter = 0;
        loop {
            let _elapsed = timer.tick().await.unwrap();
            let msg = r2r::std_msgs::msg::String {
                data: format!("Hello, world! ({})", counter),
            };
            publisher.publish(&msg).unwrap();
            counter += 1;
        }
    })?;

    // Main loop spins ros.
    loop {
        node.spin_once(std::time::Duration::from_millis(100));
        pool.run_until_stalled();
    }
}

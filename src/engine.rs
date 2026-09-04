use std::{
    io::Read,
    net::{TcpListener, TcpStream},
};

use crate::model::Order;

pub(crate) fn engine() {
    println!("this is the engine");

    let listener = TcpListener::bind("127.0.0.1:80").unwrap();
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                handle_conn(stream);
            }
            Err(e) => {
                eprintln!("ERROR: error in reading tcp stream {e} ")
            }
        }
    }
}

pub fn handle_conn(mut stream: TcpStream) {
    const BUF_SIZE: usize = std::mem::size_of::<Order>();
    let mut buf = [0u8; BUF_SIZE];

    match stream.read_exact(&mut buf) {
        Ok(_) => {
            let order: Order = unsafe { std::ptr::read(buf.as_ptr() as *const Order) };
        }
        Err(e) => {
            eprintln!("ERROR: this needs to be handled")
        }
    }
}

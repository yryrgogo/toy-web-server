use anyhow::Result;
use std::{env, net::Ipv4Addr, str};
use toytcp::tcp::TCP;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let addr: Ipv4Addr = args[1].parse()?;
    let port: u16 = args[2].parse()?;
    echo_server(addr, port)?;
    Ok(())
}

fn echo_server(local_addr: Ipv4Addr, local_port: u16) -> Result<()> {
    let tcp = TCP::new();
    let listening_socket = tcp.listen(local_addr, local_port)?;
    dbg!("listening...");
    loop {
        // TODO: Listening Socket は受信用ソケット、Connected Socket は送信側のソケット？
        // TODO: Listening Socket は1つのスレッドで接続を待ち続け、接続された新しいソケットは新しい別のスレッドで処理される？それとも接続されたソケットはすべて同じスレッドで処理される？
        // TODO: 接続済みの複数ソケットはどう処理されるのか理解が曖昧
        // TCPEventKind::ConnectionCompleted が発火したら、後続処理が行われる
        let connected_socket = tcp.accept(listening_socket)?;
        dbg!("accepted!", connected_socket.1, connected_socket.3);
        let cloned_tcp = tcp.clone();

        std::thread::spawn(move || {
            let mut buffer = [0; 1024];
            loop {
                let nbytes = cloned_tcp.recv(connected_socket, &mut buffer).unwrap();
                if nbytes == 0 {
                    return;
                }
                print!("> {}", str::from_utf8(&buffer[..nbytes]).unwrap());
                cloned_tcp
                    .send(connected_socket, &buffer[..nbytes])
                    .unwrap();
            }
        });
    }
}

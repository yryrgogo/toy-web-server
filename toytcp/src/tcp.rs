use crate::packet::TCPPacket;
use crate::socket::{SockID, Socket, TcpStatus};
use crate::tcpflags;
use anyhow::{Context, Result};
use pnet::packet::{ip::IpNextHeaderProtocols, tcp::TcpPacket, Packet};
use pnet::transport::{self, TransportChannelType};
use rand::{rngs::ThreadRng, Rng};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockWriteGuard};
use std::time::{Duration, SystemTime};
use std::{cmp, ops::Range, str, thread};

const UNDETERMINED_IP_ADDR: std::net::Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);
const UNDETERMINED_PORT: u16 = 0;
const MAX_TRANSMISSION: u8 = 5;
const RETRANSMISSION_TIMEOUT: u64 = 3;
const MSS: usize = 1460;
const PORT_RANGE: Range<u16> = 40000..60000;

pub struct TCP {
    sockets: RwLock<HashMap<SockID, Socket>>,
    event_condvar: (Mutex<Option<TCPEvent>>, Condvar),
}

#[derive(Debug, Clone, PartialEq)]
struct TCPEvent {
    sock_id: SockID, // イベント発生元のソケット ID
    kind: TCPEventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TCPEventKind {
    ConnectionCompleted,
    Acked,
    DataArrived,
    ConnectionClosed,
}

impl TCPEvent {
    fn new(sock_id: SockID, kind: TCPEventKind) -> Self {
        Self { sock_id, kind }
    }
}

impl TCP {
    pub fn new() -> Arc<Self> {
        let sockets = RwLock::new(HashMap::new());
        let tcp = Arc::new(Self {
            sockets,
            event_condvar: (Mutex::new(None), Condvar::new()),
        });

        let cloned_tcp = tcp.clone();
        std::thread::spawn(move || {
            cloned_tcp.receive_handler().unwrap();
        });

        let cloned_tcp = tcp.clone();
        std::thread::spawn(move || {
            cloned_tcp.timer();
        });

        tcp
    }

    /// thread function for timer
    /// Look at the retransmission queue for all sockets and retransmit packet that have timed out
    fn timer(&self) {
        dbg!("begin timer thread");
        loop {
            let mut table = self.sockets.write().unwrap();
            for (sock_id, socket) in table.iter_mut() {
                while let Some(mut item) = socket.retransmission_queue.pop_front() {
                    // removing acknowledged seguments from the retransmission queue
                    // required to remove segments sent when not in established state
                    if socket.send_param.unacked_seq > item.packet.get_seq() {
                        dbg!("successfully acked", item.packet.get_seq());
                        // Restore the window size by the amount of data received
                        socket.send_param.window += item.packet.payload().len() as u16;
                        self.publish_event(*sock_id, TCPEventKind::Acked);
                        continue;
                    }
                    // check if timeout
                    if item.latest_transmission_time.elapsed().unwrap()
                        < Duration::from_secs(RETRANSMISSION_TIMEOUT)
                    {
                        // if the retrieved entry has not timed out, then subsequent entries in the queue have not timed out either
                        // return to the head
                        socket.retransmission_queue.push_front(item);
                        break;
                    }
                    // if not acknowledged and timed out, retransmit
                    if item.transmission_count < MAX_TRANSMISSION {
                        dbg!("retransmit");
                        socket
                            .sender
                            .send_to(item.packet.clone(), IpAddr::V4(socket.remote_addr))
                            .context("failed to retransmit")
                            .unwrap();
                        item.transmission_count += 1;
                        item.latest_transmission_time = SystemTime::now();
                        socket.retransmission_queue.push_back(item);
                        break;
                    } else {
                        dbg!("reached MAX_TRANSMISSION");
                    }
                }
            }
            drop(table);
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn listen(&self, local_addr: Ipv4Addr, local_port: u16) -> Result<SockID> {
        // TODO: 接続先情報はいつ Socket にセットされる？
        let socket = Socket::new(
            local_addr,
            UNDETERMINED_IP_ADDR, // まだ接続先 IP アドレスは未定
            local_port,
            UNDETERMINED_PORT, // まだ接続先ポート番号は未定
            TcpStatus::Listen,
        )?;
        let mut lock = self.sockets.write().unwrap();
        let sock_id = socket.get_sock_id();
        lock.insert(sock_id, socket);
        Ok(sock_id)
    }

    pub fn accept(&self, sock_id: SockID) -> Result<SockID> {
        self.wait_event(sock_id, TCPEventKind::ConnectionCompleted);

        let mut table = self.sockets.write().unwrap();
        Ok(table
            .get_mut(&sock_id)
            .context(format!("no such socket: {:?}", sock_id))?
            .connected_connection_queue
            .pop_front()
            .context("no connected socket")?)
    }

    fn select_unused_port(&self, rng: &mut ThreadRng) -> Result<u16> {
        for _ in 0..(PORT_RANGE.end - PORT_RANGE.start) {
            let local_port = rng.gen_range(PORT_RANGE);
            let table = self.sockets.read().unwrap();
            if table.keys().all(|k| local_port != k.2) {
                return Ok(local_port);
            }
        }
        anyhow::bail!("no available port found.");
    }

    /// ターゲットに接続し、接続済みソケットの ID を返す
    pub fn connect(&self, addr: Ipv4Addr, port: u16) -> Result<SockID> {
        let mut rng = rand::thread_rng();
        let mut socket = Socket::new(
            get_source_addr_to(addr)?,
            addr,
            self.select_unused_port(&mut rng)?,
            port,
            TcpStatus::SynSent,
        )?;
        socket.send_param.initial_seq = rng.gen_range(1..1 << 31);
        socket.send_tcp_packet(socket.send_param.initial_seq, 0, tcpflags::SYN, &[])?;
        socket.send_param.unacked_seq = socket.send_param.initial_seq;
        socket.send_param.next = socket.send_param.initial_seq + 1;
        let mut table = self.sockets.write().unwrap();
        let sock_id = socket.get_sock_id();
        table.insert(sock_id, socket);
        // ロックを外してイベント待機、受信スレッドがロックを取得できるようにする
        drop(table);
        self.wait_event(sock_id, TCPEventKind::ConnectionCompleted);
        Ok(sock_id)
    }

    /// Buffer のデータを送信する。MSS 以上のデータを送信すると、複数の TCP パケットに分割される
    /// 全て送信したら、ack を待たず return
    /// TODO: 送信用スレッドで動く関数？
    pub fn send(&self, sock_id: SockID, buffer: &[u8]) -> Result<()> {
        let mut cursor = 0;
        while cursor < buffer.len() {
            let mut table = self.sockets.write().unwrap();
            // これは送信先のソケット
            let mut socket = table
                .get_mut(&sock_id)
                .context(format!("no such socket: {:?}", sock_id))?;

            let mut send_size = cmp::min(
                MSS,
                cmp::min(socket.send_param.window as usize, buffer.len() - cursor),
            );
            // TODO: マイナスになることはないの？
            while send_size == 0 {
                dbg!("unable to slide send window");
                // Remove locks, wait for events and allow receiving threads to aquire locks
                drop(table);
                self.wait_event(sock_id, TCPEventKind::Acked);
                table = self.sockets.write().unwrap();
                socket = table
                    .get_mut(&sock_id)
                    .context(format!("no such socket: {:?}", sock_id))?;

                send_size = cmp::min(
                    MSS,
                    cmp::min(socket.send_param.window as usize, buffer.len() - cursor),
                );
            }
            dbg!("current window size", socket.send_param.window);

            socket.send_tcp_packet(
                // TODO: このシーケンス番号は現在のパケットの先頭の番号、であってる？
                socket.send_param.next,
                // NOTE: acknowledge number は接続先ソケットの unacknowledge sequence number にセットされる
                // TODO: acknowledge number はこのパケットに対する返信の先頭のシーケンス番号、であってる？
                socket.recv_param.next,
                tcpflags::ACK,
                &buffer[cursor..cursor + send_size],
            )?;
            cursor += send_size;
            socket.send_param.next += send_size as u32;
            // NOTE: この window って値元に戻らないの？マイナスされたまま？
            // -> timer で ack されたセグメントをキューから削除する際に、window も更新する
            socket.send_param.window = cmp::max(socket.send_param.window - send_size as u16, 0);

            // This is unlocked for a short while and waits, allowing the receiving thread to receive ACKs
            // Keep sending until send_window reaches 0, to reduce the probability of the transmission being blocked
            drop(table);
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    /// Reads data into the buffer and returns the size read. Returns 0 if FIN is received.
    /// Block thread until packet is received.
    /// buffer is temp buffer for reading data. It is not used for storing data.
    pub fn recv(&self, sock_id: SockID, buffer: &mut [u8]) -> Result<usize> {
        let mut table = self.sockets.write().unwrap();
        let mut socket = table
            .get_mut(&sock_id)
            .context(format!("no such socket: {:?}", sock_id))?;
        // recv_param.window は recv_buffer のサイズ。よって received_size は最初は 0 になる
        let mut received_size = socket.recv_buffer.len() - socket.recv_param.window as usize;

        // if the socket receive buffer size is exceeded, wait until the receive buffer is freed
        while received_size == 0 {
            // Unlock and wait for events to allow the receiving thread to acquire the lock
            drop(table);
            dbg!("waiting incoming data");
            self.wait_event(sock_id, TCPEventKind::DataArrived);
            table = self.sockets.write().unwrap();
            socket = table
                .get_mut(&sock_id)
                .context(format!("no such socket: {:?}", sock_id))?;
            // If the thread receives a DataArrivedEvent, the socket buffer size is guaranteed to be greater than 0
            received_size = socket.recv_buffer.len() - socket.recv_param.window as usize;
        }
        // buffer が足りない場合、次の recv() で残りを読み込む。recv は loop の中で実行されているため、イベントの発火は不要
        let copy_size = cmp::min(buffer.len(), received_size);
        buffer[..copy_size].copy_from_slice(&socket.recv_buffer[..copy_size]);
        socket.recv_buffer.copy_within(copy_size.., 0);
        // buffer が足りなくて copy_size < received_size だった場合、(receive_buffer.len() - socket.recv_param.window) > 0 となるため、再度の DataArrived は wait せず、残りのデータを読み込みにいく
        socket.recv_param.window += copy_size as u16;
        Ok(copy_size)
    }

    pub fn close(&self, sock_id: SockID) -> Result<()> {
        let mut table = self.sockets.write().unwrap();
        let mut socket = table
            .get_mut(&sock_id)
            .context(format!("no such socket: {:?}", sock_id))?;
        socket.send_tcp_packet(
            socket.send_param.next,
            socket.recv_param.next,
            tcpflags::FIN | tcpflags::ACK,
            &[],
        )?;
        socket.send_param.next += 1;
        match socket.status {
            TcpStatus::Established => {
                socket.status = TcpStatus::FinWait1;
                drop(table);
                self.wait_event(sock_id, TCPEventKind::ConnectionClosed);

                let mut table = self.sockets.write().unwrap();
                table.remove(&sock_id);
                dbg!("closed & removed", sock_id);
            }
            TcpStatus::CloseWait => {
                socket.status = TcpStatus::LastAck;
                drop(table);
                self.wait_event(sock_id, TCPEventKind::ConnectionClosed);

                let mut table = self.sockets.write().unwrap();
                table.remove(&sock_id);
                dbg!("closed & removed", sock_id);
            }
            TcpStatus::Listen => {
                table.remove(&sock_id);
            }
            _ => return Ok(()),
        }
        Ok(())
    }

    fn receive_handler(&self) -> Result<()> {
        dbg!("begin recv thread");
        let (_, mut receiver) = transport::transport_channel(
            65535,
            TransportChannelType::Layer3(IpNextHeaderProtocols::Tcp), // Ip Address が必要なため Ip Packet Level で取得
        )
        .unwrap();
        let mut packet_iter = transport::ipv4_packet_iter(&mut receiver);

        loop {
            let (packet, remote_addr) = match packet_iter.next() {
                Ok((p, r)) => (p, r),
                Err(_) => continue,
            };
            let local_addr = packet.get_destination();
            // pnet の TcpPacket を生成
            let tcp_packet = match TcpPacket::new(packet.payload()) {
                Some(p) => p,
                None => {
                    continue;
                }
            };
            // pnet の TcpPacket から tcp::TCPPacket に変換
            let packet = TCPPacket::from(tcp_packet);
            let remote_addr = match remote_addr {
                IpAddr::V4(addr) => addr,
                _ => {
                    continue;
                }
            };
            let mut table = self.sockets.write().unwrap();
            let socket = match table.get_mut(&SockID(
                local_addr,
                remote_addr,
                packet.get_dest(),
                packet.get_src(),
            )) {
                Some(socket) => socket, // 接続済みソケット
                None => match table.get_mut(&SockID(
                    local_addr,
                    UNDETERMINED_IP_ADDR,
                    packet.get_dest(),
                    UNDETERMINED_PORT,
                )) {
                    Some(socket) => socket, // Listening Socket
                    None => continue,       // どのソケットにも該当しない場合は無視
                },
            };

            if !packet.is_correct_checksum(local_addr, remote_addr) {
                dbg!("invalid checksum");
                continue;
            }
            let sock_id = socket.get_sock_id();
            if let Err(error) = match socket.status {
                TcpStatus::Listen => self.listen_handler(table, sock_id, &packet, remote_addr),
                TcpStatus::SynRcvd => self.synrcvd_handler(table, sock_id, &packet),
                TcpStatus::SynSent => self.synsent_handler(socket, &packet),
                TcpStatus::Established => self.established_handler(socket, &packet),
                TcpStatus::CloseWait | TcpStatus::LastAck => self.close_handler(socket, &packet),
                TcpStatus::FinWait1 | TcpStatus::FinWait2 => self.finwait_handler(socket, &packet),
                _ => {
                    dbg!("not implemented state");
                    Ok(())
                }
            } {
                dbg!(error);
            }
        }
    }

    fn delete_acked_segment_from_retransmission_queue(&self, socket: &mut Socket) {
        dbg!("ack accept", socket.send_param.unacked_seq);
        while let Some(item) = socket.retransmission_queue.pop_front() {
            if socket.send_param.unacked_seq > item.packet.get_seq() {
                // ack されてるので除去
                dbg!("successfully acked", item.packet.get_seq());
                socket.send_param.window += item.packet.payload().len() as u16;
                self.publish_event(socket.get_sock_id(), TCPEventKind::Acked);
            } else {
                // ack されてない、キューに戻す
                socket.retransmission_queue.push_front(item);
                break;
            }
        }
    }

    fn synsent_handler(&self, socket: &mut Socket, packet: &TCPPacket) -> Result<()> {
        dbg!("synsent handler");
        if packet.get_flag() & tcpflags::ACK > 0
            && socket.send_param.unacked_seq <= packet.get_ack()
            && packet.get_ack() <= socket.send_param.next
            && packet.get_flag() & tcpflags::SYN > 0
        {
            socket.recv_param.next = packet.get_seq() + 1;
            socket.recv_param.initial_seq = packet.get_seq();
            socket.send_param.unacked_seq = packet.get_ack();
            socket.send_param.window = packet.get_window_size();
            if socket.send_param.unacked_seq > socket.send_param.initial_seq {
                socket.status = TcpStatus::Established;
                socket.send_tcp_packet(
                    socket.send_param.next,
                    socket.recv_param.next,
                    tcpflags::ACK,
                    &[],
                )?;
                dbg!("status: synsent ->", &socket.status);
                self.publish_event(socket.get_sock_id(), TCPEventKind::ConnectionCompleted);
            }
            // TODO: send_param.initial_seq >= send_param.unacked_seq となるのは、初回のアクティブオープン時？
            else {
                socket.status = TcpStatus::SynRcvd;
                socket.send_tcp_packet(
                    socket.send_param.next,
                    socket.recv_param.next,
                    tcpflags::ACK,
                    &[],
                )?;
                dbg!("status: synsent ->", &socket.status);
            }
        }
        Ok(())
    }

    fn listen_handler(
        &self,
        mut table: RwLockWriteGuard<HashMap<SockID, Socket>>,
        listening_socket_id: SockID,
        packet: &TCPPacket,
        remote_addr: Ipv4Addr,
    ) -> Result<()> {
        dbg!("listen handler");
        if packet.get_flag() & tcpflags::ACK > 0 {
            // 本来なら RST を send する
            return Ok(());
        }

        let listening_socket = table.get_mut(&listening_socket_id).unwrap();
        if packet.get_flag() & tcpflags::SYN > 0 {
            let mut connection_socket = Socket::new(
                listening_socket.local_addr,
                remote_addr,
                listening_socket.local_port,
                packet.get_src(),
                TcpStatus::SynRcvd,
            )?;
            connection_socket.recv_param.next = packet.get_seq() + 1;
            connection_socket.recv_param.initial_seq = packet.get_seq();
            connection_socket.send_param.initial_seq = rand::thread_rng().gen_range(1..1 << 31);
            connection_socket.send_param.window = packet.get_window_size();
            connection_socket.send_tcp_packet(
                connection_socket.send_param.initial_seq,
                connection_socket.recv_param.next,
                tcpflags::SYN | tcpflags::ACK,
                &[],
            )?;
            connection_socket.send_param.next = connection_socket.send_param.initial_seq + 1;
            connection_socket.send_param.unacked_seq = connection_socket.send_param.initial_seq;
            // NOTE: なぜ listening_socket_id を保持するのか？どの Listening Socket で待機していたかを知る必要がある？
            // -> syncrcvd_handler で接続が確立したとき、Listening Socket の queue に接続完了した Socket ID を追加するため
            connection_socket.listening_socket = Some(listening_socket.get_sock_id());
            dbg!("status: listen ->", &connection_socket.status);
            table.insert(connection_socket.get_sock_id(), connection_socket);
        }
        Ok(())
    }

    // Passive Open
    fn synrcvd_handler(
        &self,
        mut table: RwLockWriteGuard<HashMap<SockID, Socket>>,
        sock_id: SockID,
        packet: &TCPPacket,
    ) -> Result<()> {
        dbg!("synrcvd handler");
        let socket = table.get_mut(&sock_id).unwrap();

        if packet.get_flag() & tcpflags::ACK > 0
            && socket.send_param.unacked_seq <= packet.get_ack()
            && packet.get_ack() <= socket.send_param.next
        {
            // Passive Open の際に Active Open する側から送られてくるパケットのシーケンス番号は、最初のパケットのシーケンス番号になっている（Three-Way Handshake の Active Open 側の ACK パケットはシーケンス番号にプラスされない？）
            socket.recv_param.next = packet.get_seq();
            // Passive Open で最初に受け取る Acknowledge Number はシーケンス番号+1
            socket.send_param.unacked_seq = packet.get_ack();
            socket.status = TcpStatus::Established;
            dbg!("status: synrcvd ->", &socket.status);
            if let Some(id) = socket.listening_socket {
                let listening_socket = table.get_mut(&id).unwrap();
                listening_socket
                    .connected_connection_queue
                    .push_back(sock_id);
                self.publish_event(
                    listening_socket.get_sock_id(),
                    TCPEventKind::ConnectionCompleted,
                );
            }
        }
        Ok(())
    }

    fn established_handler(&self, socket: &mut Socket, packet: &TCPPacket) -> Result<()> {
        dbg!("established handler");
        if socket.send_param.unacked_seq < packet.get_ack()
            && packet.get_ack() <= socket.send_param.next
        {
            socket.send_param.unacked_seq = packet.get_ack();
            self.delete_acked_segment_from_retransmission_queue(socket);
        } else if socket.send_param.next < packet.get_ack() {
            // 未送信セグメントに対する ack は破棄
            return Ok(());
        }
        if packet.get_flag() & tcpflags::ACK == 0 {
            // ACK が立っていないパケットは破棄
            return Ok(());
        }
        if !packet.payload().is_empty() {
            self.process_payload(socket, &packet)?;
        }
        Ok(())
    }

    /// TODO: 各パラメータの意味を理解する
    fn process_payload(&self, socket: &mut Socket, packet: &TCPPacket) -> Result<()> {
        // 受信待ちの状態でパケットが送られてきた段階なら、socket.recv_param.next は packet.get_seq() と同じ値になっているはず？
        // パケット受信後に、そのパケットの次に受け取るべきシーケンス番号を socket.recv_param.next にセットしているなら、offset はマイナスになる
        // packet.get_seq() > socket.recv_param.next はありえない。packet.get_seq() の方が大きいと、途中のパケットが抜けてるから
        // socket.recv_param.window の初期値 = recv_buffer.len()
        // オフセットは、パケットが分割して送られてきて、到着した順序が正しくない場合に、受信バッファのどこにセットするかを決めるために使う
        // TODO: (packet.get_seq() - socket.recv_param.next) の必要性が不明。この値がプラスになるとき、マイナスになる時はどんな時か？そもそもあり得るのか？は、各ケースの状態を一つ一つ書き出してみないとわからない
        let offset = socket.recv_buffer.len() - socket.recv_param.window as usize
            + (packet.get_seq() - socket.recv_param.next) as usize;

        // TODO: 受け取った payload 分の buffer が残ってなかったとき、buffer に入りきらなかった分はどうなるか？
        // TODO: packet.payload の copy_size 以降を前に持ってくるという操作はどこにもないため、取り出せないのでは？そもそも MSS < RECEIVE_BUFFER_SIZE という前提がある？（普通そうだと思うが）
        let copy_size = cmp::min(packet.payload().len(), socket.recv_buffer.len() - offset);

        // ソケットの受信バッファへの書き込みはここで行われる
        // payload の中身にアクセスするのはここのみ
        socket.recv_buffer[offset..offset + copy_size]
            .copy_from_slice(&packet.payload()[..copy_size]);

        // TODO: わからん -> ロス再送の際に穴埋めされるため max をとる
        // TODO: 具体のユースケースを元に考える必要がある
        socket.recv_param.tail =
            cmp::max(socket.recv_param.tail, packet.get_seq() + copy_size as u32);

        if packet.get_seq() == socket.recv_param.next {
            // 順序入れ替わりなしの場合のみ recv_param.next を進められる
            socket.recv_param.next = socket.recv_param.tail;
            // TODO: 受け取ったパケットのサイズ分だけ window を減らす
            socket.recv_param.window -= (socket.recv_param.tail - packet.get_seq()) as u16;
        }

        if copy_size > 0 {
            // Successfully copied to receive buffer
            socket.send_tcp_packet(
                socket.send_param.next,
                socket.recv_param.next,
                tcpflags::ACK,
                &[],
            )?;
        } else {
            // When the receive buffer overflows, the segment is discarded
            dbg!("recv buffer overflow");
        }
        self.publish_event(socket.get_sock_id(), TCPEventKind::DataArrived);
        Ok(())
    }

    fn finwait_handler(&self, socket: &mut Socket, packet: &TCPPacket) -> Result<()> {
        dbg!("finwait handler");
        if socket.send_param.unacked_seq < packet.get_ack()
            && packet.get_ack() <= socket.send_param.next
        {
            socket.send_param.unacked_seq = packet.get_ack();
            self.delete_acked_segment_from_retransmission_queue(socket);
        } else if socket.send_param.next < packet.get_ack() {
            // 未送信セグメントに対する ACK は破棄
            return Ok(());
        }
        if packet.get_flag() & tcpflags::ACK == 0 {
            // ACK が立っていないパケットは破棄
            return Ok(());
        }
        if !packet.payload().is_empty() {
            self.process_payload(socket, &packet)?;
        }
    }

    /// 指定した Socket ID と TCPEventKind に一致するイベントが発生するまで待機
    fn wait_event(&self, sock_id: SockID, kind: TCPEventKind) {
        let (lock, cvar) = &self.event_condvar;
        let mut event = lock.lock().unwrap();
        loop {
            if let Some(ref e) = *event {
                if e.sock_id == sock_id && e.kind == kind {
                    break;
                }
            }
            // cvar が notify されるまで event のロックを外して待機
            event = cvar.wait(event).unwrap();
        }
        dbg!(&event);
        *event = None;
    }

    /// 指定の Socket ID イベントを発行する
    fn publish_event(&self, sock_id: SockID, kind: TCPEventKind) {
        let (lock, cvar) = &self.event_condvar;
        let mut e = lock.lock().unwrap();
        *e = Some(TCPEvent::new(sock_id, kind));
        cvar.notify_all();
    }
}

fn get_source_addr_to(addr: Ipv4Addr) -> Result<Ipv4Addr> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("ip route get {} | grep src", addr))
        .output()?;
    let mut output = str::from_utf8(&output.stdout)?
        .trim()
        .split_ascii_whitespace();
    while let Some(s) = output.next() {
        if s == "src" {
            break;
        }
    }
    let ip = output.next().context("failed to get src ip")?;
    dbg!("source addr", ip);
    ip.parse().context("failed to parse source ip")
}

use crate::codec::{decode_packet, encode_packet};
use crate::packet::Packet;
use crate::version::{ProtocolError, ProtocolResult};
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// Abstract network transport for packet transmission.
pub trait Transport: Send {
    /// Send a packet over the transport.
    fn send(&mut self, packet: Packet) -> ProtocolResult<()>;
    /// Poll for an incoming packet without blocking.
    fn recv(&mut self) -> ProtocolResult<Option<Packet>>;
    /// Check whether the transport channel is open and connected.
    fn is_connected(&self) -> bool;
    /// Explicitly close the transport.
    fn close(&mut self);
}

/// In-memory loopback transport for single-player games and local testing.
pub struct LoopbackTransport {
    sender: Option<Sender<Packet>>,
    receiver: Receiver<Packet>,
    connected: bool,
}

impl LoopbackTransport {
    /// Creates a bidirectional loopback transport pair: (client_end, server_end).
    pub fn create_pair() -> (Self, Self) {
        let (tx_a, rx_b) = mpsc::channel();
        let (tx_b, rx_a) = mpsc::channel();

        let end_a = LoopbackTransport {
            sender: Some(tx_a),
            receiver: rx_a,
            connected: true,
        };

        let end_b = LoopbackTransport {
            sender: Some(tx_b),
            receiver: rx_b,
            connected: true,
        };

        (end_a, end_b)
    }
}

impl Transport for LoopbackTransport {
    fn send(&mut self, packet: Packet) -> ProtocolResult<()> {
        if !self.connected {
            return Err(ProtocolError::NotConnected);
        }
        if let Some(sender) = &self.sender {
            sender
                .send(packet)
                .map_err(|_| ProtocolError::ConnectionClosed)
        } else {
            Err(ProtocolError::ConnectionClosed)
        }
    }

    fn recv(&mut self) -> ProtocolResult<Option<Packet>> {
        if !self.connected {
            return Ok(None);
        }
        match self.receiver.try_recv() {
            Ok(packet) => Ok(Some(packet)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.connected = false;
                Ok(None)
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn close(&mut self) {
        self.connected = false;
        self.sender = None;
    }
}

/// Transmitter half of an abstract network transport.
pub trait TransportSend: Send + 'static {
    /// Send a packet over the transport.
    fn send(&mut self, packet: Packet) -> ProtocolResult<()>;
    /// Check whether the transport channel is open and connected.
    fn is_connected(&self) -> bool;
    /// Explicitly close the transport.
    fn close(&mut self);
}

/// Receiver half of an abstract network transport.
pub trait TransportRecv: Send + 'static {
    /// Poll for an incoming packet without blocking.
    fn recv(&mut self) -> ProtocolResult<Option<Packet>>;
    /// Check whether the transport channel is open and connected.
    fn is_connected(&self) -> bool;
    /// Explicitly close the transport.
    fn close(&mut self);
}

/// In-memory loopback sender half.
pub struct LoopbackSender {
    sender: Option<Sender<Packet>>,
    connected: bool,
}

impl TransportSend for LoopbackSender {
    fn send(&mut self, packet: Packet) -> ProtocolResult<()> {
        if !self.connected {
            return Err(ProtocolError::NotConnected);
        }
        if let Some(sender) = &self.sender {
            sender
                .send(packet)
                .map_err(|_| ProtocolError::ConnectionClosed)
        } else {
            Err(ProtocolError::ConnectionClosed)
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn close(&mut self) {
        self.connected = false;
        self.sender = None;
    }
}

/// In-memory loopback receiver half.
pub struct LoopbackReceiver {
    receiver: Receiver<Packet>,
    connected: bool,
}

impl TransportRecv for LoopbackReceiver {
    fn recv(&mut self) -> ProtocolResult<Option<Packet>> {
        if !self.connected {
            return Ok(None);
        }
        match self.receiver.try_recv() {
            Ok(packet) => Ok(Some(packet)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                self.connected = false;
                Ok(None)
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn close(&mut self) {
        self.connected = false;
    }
}

/// UDP sender half.
pub struct UdpSender {
    socket: UdpSocket,
    remote_addr: Option<SocketAddr>,
    connected: bool,
}

impl TransportSend for UdpSender {
    fn send(&mut self, packet: Packet) -> ProtocolResult<()> {
        if !self.connected {
            return Err(ProtocolError::NotConnected);
        }
        let remote = self.remote_addr.ok_or_else(|| {
            ProtocolError::TransportError("No remote destination address set".to_string())
        })?;

        let bytes = encode_packet(&packet);
        self.socket
            .send_to(&bytes, remote)
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn close(&mut self) {
        self.connected = false;
        self.remote_addr = None;
    }
}

impl UdpSender {
    pub fn set_remote_addr(&mut self, addr: SocketAddr) {
        self.remote_addr = Some(addr);
    }
}

/// UDP receiver half.
pub struct UdpReceiver {
    socket: UdpSocket,
    connected: bool,
    buffer: [u8; 65535],
    last_peer: Option<SocketAddr>,
}

impl TransportRecv for UdpReceiver {
    fn recv(&mut self) -> ProtocolResult<Option<Packet>> {
        if !self.connected {
            return Ok(None);
        }

        match self.socket.recv_from(&mut self.buffer) {
            Ok((size, peer)) => {
                self.last_peer = Some(peer);
                let packet = decode_packet(&self.buffer[..size])?;
                Ok(Some(packet))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(ProtocolError::TransportError(e.to_string())),
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn close(&mut self) {
        self.connected = false;
    }
}

impl UdpReceiver {
    pub fn last_peer(&self) -> Option<SocketAddr> {
        self.last_peer
    }
}

/// UDP socket transport for remote client/server multiplayer networking.
pub struct UdpTransport {
    socket: UdpSocket,
    remote_addr: Option<SocketAddr>,
    connected: bool,
    buffer: [u8; 65535],
}

impl UdpTransport {
    /// Bind a UDP socket to a local address.
    pub fn bind(local_addr: &str) -> ProtocolResult<Self> {
        let socket = UdpSocket::bind(local_addr)
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;

        Ok(UdpTransport {
            socket,
            remote_addr: None,
            connected: true,
            buffer: [0u8; 65535],
        })
    }

    /// Set the destination remote address for peer-to-peer or client-to-server transmission.
    pub fn connect_to(&mut self, remote_addr: &str) -> ProtocolResult<()> {
        let addr: SocketAddr = remote_addr
            .parse()
            .map_err(|e: std::net::AddrParseError| ProtocolError::TransportError(e.to_string()))?;
        self.remote_addr = Some(addr);
        Ok(())
    }

    pub fn local_addr(&self) -> ProtocolResult<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| ProtocolError::TransportError(e.to_string()))
    }

    /// Split UDP transport into distinct sender and receiver handles for multithreaded I/O.
    pub fn split(self) -> ProtocolResult<(UdpSender, UdpReceiver)> {
        let sender_socket = self
            .socket
            .try_clone()
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;

        Ok((
            UdpSender {
                socket: sender_socket,
                remote_addr: self.remote_addr,
                connected: self.connected,
            },
            UdpReceiver {
                socket: self.socket,
                connected: self.connected,
                buffer: [0u8; 65535],
                last_peer: None,
            },
        ))
    }
}

impl LoopbackTransport {
    /// Split loopback transport into distinct sender and receiver handles for multithreaded testing.
    pub fn split(self) -> (LoopbackSender, LoopbackReceiver) {
        (
            LoopbackSender {
                sender: self.sender,
                connected: self.connected,
            },
            LoopbackReceiver {
                receiver: self.receiver,
                connected: self.connected,
            },
        )
    }
}

impl Transport for UdpTransport {
    fn send(&mut self, packet: Packet) -> ProtocolResult<()> {
        if !self.connected {
            return Err(ProtocolError::NotConnected);
        }
        let remote = self.remote_addr.ok_or_else(|| {
            ProtocolError::TransportError("No remote destination address set".to_string())
        })?;

        let bytes = encode_packet(&packet);
        self.socket
            .send_to(&bytes, remote)
            .map_err(|e| ProtocolError::TransportError(e.to_string()))?;

        Ok(())
    }

    fn recv(&mut self) -> ProtocolResult<Option<Packet>> {
        if !self.connected {
            return Ok(None);
        }

        match self.socket.recv_from(&mut self.buffer) {
            Ok((size, peer)) => {
                if self.remote_addr.is_none() {
                    self.remote_addr = Some(peer);
                }
                let packet = decode_packet(&self.buffer[..size])?;
                Ok(Some(packet))
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(ProtocolError::TransportError(e.to_string())),
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn close(&mut self) {
        self.connected = false;
        self.remote_addr = None;
    }
}

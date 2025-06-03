use crate::ctypes::{AF_INET, in_addr, sockaddr_in};
use core::net::{Ipv4Addr, SocketAddrV4};

impl From<SocketAddrV4> for sockaddr_in {
    fn from(addr: SocketAddrV4) -> sockaddr_in {
        sockaddr_in {
            sin_family: AF_INET as u16,
            sin_port: addr.port().to_be(),
            sin_addr: in_addr {
                // `s_addr` is stored as BE on all machines and the array is in BE order.
                // So the native endian conversion method is used so that it's never swapped.
                s_addr: u32::from_ne_bytes(addr.ip().octets()),
            },
            sin_zero: [0; 8],
        }
    }
}

impl From<sockaddr_in> for SocketAddrV4 {
    fn from(addr: sockaddr_in) -> SocketAddrV4 {
        SocketAddrV4::new(
            Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
            u16::from_be(addr.sin_port),
        )
    }
}

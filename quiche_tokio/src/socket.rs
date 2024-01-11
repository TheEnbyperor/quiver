pub struct UdpSocket {
    fd: libc::c_int,
}

impl UdpSocket {
    pub async fn new(bind: std::net::SocketAddr) -> std::io::Result<Self> {
        let inet = match bind {
            std::net::SocketAddr::V4(_) => libc::AF_INET,
            std::net::SocketAddr::V6(_) => libc::AF_INET6,
        };

        let fd = unsafe {
            libc::socket(inet, libc::SOCK_DGRAM, 0)
        };
        if fd == -1 {
            return Err(std::io::Error::last_os_error());
        }

        let sock_addr = socket_addr_to_c(bind);
        if unsafe {
            libc::bind(
                fd, std::mem::transmute(&sock_addr),
                std::mem::size_of::<libc::sockaddr>() as libc::socklen_t
            )
        } == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            fd
        })
    }

    pub fn local_addr(&self) -> std::io::Result<()> {
        let mut local_address: std::mem::MaybeUninit<CSocketAddr> = std::mem::MaybeUninit::uninit();
        let mut address_length: libc::socklen_t = std::mem::size_of::<libc::sockaddr>() as libc::socklen_t;
        if unsafe {
            libc::getsockname(
                self.fd, std::mem::transmute(&mut local_address),
                &mut address_length
            )
        } == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(())
    }
}

#[repr(C)]
union CSocketAddr {
    v4: libc::sockaddr_in,
    v6: libc::sockaddr_in6,
}

fn socket_addr_to_c(addr: std::net::SocketAddr) -> CSocketAddr {
    match addr {
        std::net::SocketAddr::V4(v4) => {
            CSocketAddr {
                v4: libc::sockaddr_in {
                    sin_len: 0,
                    sin_family: libc::AF_INET as u8,
                    sin_port: v4.port(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(v4.ip().octets())
                    },
                    sin_zero: [0; 8],
                }
            }
        }
        std::net::SocketAddr::V6(v6) => {
            CSocketAddr {
                v6 :libc::sockaddr_in6 {
                    sin6_len: 0,
                    sin6_family: libc::AF_INET6 as u8,
                    sin6_port: v6.port(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: v6.ip().octets()
                    },
                    sin6_flowinfo: 0,
                    sin6_scope_id: 0,
                }
            }
        }
    }
}
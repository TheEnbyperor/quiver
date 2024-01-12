use std::os::fd::{AsRawFd, FromRawFd};

pub struct UdpSocket {
    fd: async_io::Async<std::os::fd::OwnedFd>,
    has_pacing: bool,
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
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) };

        let res = unsafe {
            let val: libc::c_int = 1;
            libc::setsockopt(
                fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEADDR,
                std::mem::transmute(&val),
                std::mem::size_of_val(&val) as libc::socklen_t,
            )
        };
        if res < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let res = unsafe {
            let val: libc::c_int = 1;
            libc::setsockopt(
                fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_REUSEPORT,
                std::mem::transmute(&val),
                std::mem::size_of_val(&val) as libc::socklen_t,
            )
        };
        if res < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let tx_time_opts = libc::sock_txtime {
            clockid: libc::CLOCK_MONOTONIC,
            flags: 0,
        };

        let res = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_TXTIME,
                std::mem::transmute(&tx_time_opts),
                std::mem::size_of::<libc::sock_txtime>() as libc::socklen_t
            )
        };
        let has_pacing = res >= 0;

        // let segment_size_i32 = segment_size as i32;
        // let res = unsafe {
        //     libc::setsockopt(
        //         fd.as_raw_fd(), libc::SOL_UDP, libc::UDP_SEGMENT,
        //         std::mem::transmute(&segment_size_i32),
        //         std::mem::size_of::<i32>() as libc::socklen_t
        //     )
        // };
        // let has_gso = res != -1;

        let sock_addr = socket_addr_to_c(bind);
        if unsafe {
            libc::bind(
                fd.as_raw_fd(), std::mem::transmute(&sock_addr),
                std::mem::size_of::<CSocketAddr>() as libc::socklen_t
            )
        } == -1 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self {
            fd: async_io::Async::new(fd)?,
            has_pacing
        })
    }

    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        let mut local_address: std::mem::MaybeUninit<CSocketAddr> = std::mem::MaybeUninit::uninit();
        let mut address_length: libc::socklen_t = std::mem::size_of::<CSocketAddr>() as libc::socklen_t;
        if unsafe {
            libc::getsockname(
                self.fd.as_raw_fd(), std::mem::transmute(&mut local_address), &mut address_length
            )
        } == -1 {
            return Err(std::io::Error::last_os_error());
        }

        let local_address = unsafe { local_address.assume_init() };

        Ok(c_to_socket_addr(local_address))
    }

    pub fn has_pacing(&self) -> bool {
        self.has_pacing
    }

    pub async fn send_dgram(&self, buf: &[u8], send_info: &quiche::SendInfo) -> std::io::Result<usize> {
        let send_time = std_time_to_u64(&send_info.at);
        let control = [0u8; unsafe {
            libc::CMSG_SPACE(std::mem::size_of::<u64>() as u32) as usize
        }];

        let dest_addr = socket_addr_to_c(send_info.to);

        let written = self.fd.write_with(|fd| {
            let iov = libc::iovec {
                iov_base: buf.as_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            };

            let mut msg = libc::msghdr {
                msg_name: unsafe {
                    std::mem::transmute(&dest_addr)
                },
                msg_namelen: std::mem::size_of::<CSocketAddr>() as libc::socklen_t,
                msg_iov: unsafe {
                    std::mem::transmute(&iov)
                },
                msg_iovlen: 1,
                msg_control: std::ptr::null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            };

            if self.has_pacing {
                msg.msg_control = unsafe {
                    std::mem::transmute(&control)
                };
                msg.msg_controllen = std::mem::size_of_val(&control);

                let cmsg: &mut libc::cmsghdr = unsafe {
                    std::mem::transmute(libc::CMSG_FIRSTHDR(&msg))
                };
                cmsg.cmsg_level = libc::SOL_SOCKET;
                cmsg.cmsg_type = libc::SCM_TXTIME;
                cmsg.cmsg_len = unsafe {
                    libc::CMSG_LEN(std::mem::size_of::<u64>() as u32) as usize
                };
                unsafe {
                    *(libc::CMSG_DATA(cmsg) as *mut u64) = send_time;
                }
            }

            let res = unsafe {
                libc::sendmsg(fd.as_raw_fd(), &msg, libc::MSG_DONTWAIT)
            };
            if res < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(res)
        }).await?;

        Ok(written as usize)
    }

    pub async fn recv_dgram(&self, buf: &mut [u8]) -> std::io::Result<(usize, quiche::RecvInfo)> {
        let mut peer_address: std::mem::MaybeUninit<CSocketAddr> = std::mem::MaybeUninit::uninit();

        let read = self.fd.read_with(|fd| {
            let iov = libc::iovec {
                iov_base: buf.as_mut_ptr() as *mut libc::c_void,
                iov_len: buf.len(),
            };

            let mut msg = libc::msghdr {
                msg_name: unsafe {
                    std::mem::transmute(&mut peer_address)
                },
                msg_namelen: std::mem::size_of::<CSocketAddr>() as libc::socklen_t,
                msg_iov: unsafe {
                    std::mem::transmute(&iov)
                },
                msg_iovlen: 1,
                msg_control: std::ptr::null_mut(),
                msg_controllen: 0,
                msg_flags: 0,
            };

            let res = unsafe {
                libc::recvmsg(fd.as_raw_fd(), &mut msg, libc::MSG_DONTWAIT)
            };
            if res < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(res)
        }).await?;

        let peer_address = unsafe { peer_address.assume_init() };
        let peer_address = c_to_socket_addr(peer_address);

        Ok((read as usize, quiche::RecvInfo {
            from: peer_address,
            to: self.local_addr()?
        }))
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
                    sin_family: libc::AF_INET as u16,
                    sin_port: v4.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_be_bytes(v4.ip().octets()).to_be()
                    },
                    sin_zero: [0; 8],
                }
            }
        }
        std::net::SocketAddr::V6(v6) => {
            CSocketAddr {
                v6: libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as u16,
                    sin6_port: v6.port().to_be(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: v6.ip().octets()
                    },
                    sin6_flowinfo: v6.flowinfo(),
                    sin6_scope_id: v6.scope_id(),
                }
            }
        }
    }
}

fn c_to_socket_addr(addr: CSocketAddr) -> std::net::SocketAddr {
    let addr_type = unsafe { addr.v4.sin_family } as libc::c_int;
    if addr_type == libc::AF_INET {
        let addr = unsafe { addr.v4 };
        let ip = std::net::Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr as u32));
        std::net::SocketAddr::V4(
            std::net::SocketAddrV4::new(ip, u16::from_be(addr.sin_port))
        )
    } else if addr_type == libc::AF_INET6 {
        let addr = unsafe { addr.v6 };
        let ip = std::net::Ipv6Addr::from(addr.sin6_addr.s6_addr);
        std::net::SocketAddr::V6(
            std::net::SocketAddrV6::new(ip, u16::from_be(addr.sin6_port), addr.sin6_flowinfo, addr.sin6_scope_id)
        )
    } else {
        panic!("Unknown address family {}", addr_type);
    }
}

fn std_time_to_u64(time: &std::time::Instant) -> u64 {
    const NANOS_PER_SEC: u64 = 1_000_000_000;

    const INSTANT_ZERO: std::time::Instant =
        unsafe { std::mem::transmute(std::time::UNIX_EPOCH) };

    let raw_time = time.duration_since(INSTANT_ZERO);

    let sec = raw_time.as_secs();
    let nsec = raw_time.subsec_nanos();

    sec * NANOS_PER_SEC + nsec as u64
}
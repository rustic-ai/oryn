//! Descriptor attachment on a serialized, attested native broker stream. The
//! caller owns the stream request lock for metadata and this attachment.
#![cfg(unix)]
use std::{
    io, mem,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::net::UnixStream,
};
const TAG: u8 = 0x53;
pub(crate) fn send(stream: &UnixStream, request: u64, descriptor: &impl AsRawFd) -> io::Result<()> {
    let mut payload = [0; 9];
    payload[0] = TAG;
    payload[1..].copy_from_slice(&request.to_be_bytes());
    let mut vector = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    // usize storage supplies cmsghdr alignment on both supported Unix targets.
    let bytes = unsafe { libc::CMSG_SPACE(mem::size_of::<libc::c_int>() as _) as usize };
    let mut control = vec![0usize; bytes.div_ceil(mem::size_of::<usize>())];
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = bytes as _;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(mem::size_of::<libc::c_int>() as _) as _;
        std::ptr::write_unaligned(
            libc::CMSG_DATA(header).cast::<libc::c_int>(),
            descriptor.as_raw_fd(),
        );
    }
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    let sent = loop {
        let sent = unsafe { libc::sendmsg(stream.as_raw_fd(), &message, flags) };
        if sent >= 0 {
            break sent as usize;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    if sent == payload.len() {
        return Ok(());
    }
    // SCM_RIGHTS attaches to the first byte only; never resend the descriptor.
    use io::Write;
    let mut stream = stream;
    stream.write_all(&payload[sent..])
}
#[cfg(any(feature = "v8-host", test))]
pub(crate) fn receive(stream: &UnixStream, request: u64) -> io::Result<OwnedFd> {
    let mut payload = [0; 9];
    let mut vector = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    let bytes = unsafe { libc::CMSG_SPACE((8 * mem::size_of::<libc::c_int>()) as _) as usize };
    let mut control = vec![0usize; bytes.div_ceil(mem::size_of::<usize>())];
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = bytes as _;
    let received = loop {
        let received =
            unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_WAITALL) };
        if received >= 0 {
            break received as usize;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    };
    let mut descriptors = Vec::new();
    let mut invalid = false;
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            let minimum = libc::CMSG_LEN(0) as usize;
            let size = (*header).cmsg_len as usize;
            if size < minimum {
                invalid = true;
                break;
            }
            if (*header).cmsg_level != libc::SOL_SOCKET || (*header).cmsg_type != libc::SCM_RIGHTS {
                invalid = true;
            } else {
                let count = (size - minimum) / mem::size_of::<libc::c_int>();
                if !(size - minimum).is_multiple_of(mem::size_of::<libc::c_int>()) {
                    invalid = true;
                }
                for index in 0..count {
                    let fd = std::ptr::read_unaligned(
                        libc::CMSG_DATA(header).cast::<libc::c_int>().add(index),
                    );
                    if fd < 0 {
                        invalid = true;
                    } else {
                        descriptors.push(OwnedFd::from_raw_fd(fd));
                    }
                }
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    if received != payload.len()
        || payload[0] != TAG
        || payload[1..] != request.to_be_bytes()
        || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
        || invalid
        || descriptors.len() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid shared memory descriptor attachment",
        ));
    }
    let descriptor = descriptors.pop().unwrap();
    if unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(descriptor)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_transport_preserves_native_file_and_rejects_wrong_correlation() {
        let (a, b) = UnixStream::pair().unwrap();
        let file = std::fs::File::open("/dev/null").unwrap();
        send(&a, 7, &file).unwrap();
        let received = receive(&b, 7).unwrap();
        assert!(received.as_raw_fd() >= 0);
        send(&a, 8, &file).unwrap();
        assert_eq!(
            receive(&b, 9).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        send(&a, 10, &file).unwrap();
        drop(receive(&b, 10).unwrap());
    }
}

/// Buffered JSON and descriptor reader. Every kernel read uses recvmsg so a
/// read-ahead across a JSON newline cannot silently discard SCM_RIGHTS.
pub(crate) struct DescriptorReader {
    stream: UnixStream,
    buffer: Vec<u8>,
    position: usize,
    descriptors: std::collections::VecDeque<OwnedFd>,
}
impl DescriptorReader {
    pub(crate) fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            position: 0,
            descriptors: Default::default(),
        }
    }
    pub(crate) fn attachment(&mut self, request: u64) -> io::Result<OwnedFd> {
        use io::Read;
        let mut payload = [0; 9];
        self.read_exact(&mut payload)?;
        if payload[0] != TAG || payload[1..] != request.to_be_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "shared memory attachment correlation",
            ));
        }
        self.descriptors.pop_front().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "missing shared memory descriptor",
            )
        })
    }
    fn refill(&mut self) -> io::Result<()> {
        self.buffer.resize(64 * 1024, 0);
        self.position = 0;
        let mut vector = libc::iovec {
            iov_base: self.buffer.as_mut_ptr().cast(),
            iov_len: self.buffer.len(),
        };
        let bytes = unsafe { libc::CMSG_SPACE((64 * mem::size_of::<libc::c_int>()) as _) as usize };
        let mut control = vec![0usize; bytes.div_ceil(mem::size_of::<usize>())];
        let mut message: libc::msghdr = unsafe { mem::zeroed() };
        message.msg_iov = &mut vector;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = bytes as _;
        #[cfg(target_os = "linux")]
        let flags = libc::MSG_CMSG_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = 0;
        let received = loop {
            let received = unsafe { libc::recvmsg(self.stream.as_raw_fd(), &mut message, flags) };
            if received >= 0 {
                break received as usize;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        };
        self.buffer.truncate(received);
        let mut incoming = Vec::new();
        let mut invalid = false;
        unsafe {
            let mut header = libc::CMSG_FIRSTHDR(&message);
            while !header.is_null() {
                let minimum = libc::CMSG_LEN(0) as usize;
                let size = (*header).cmsg_len as usize;
                if size < minimum {
                    invalid = true;
                    break;
                }
                if (*header).cmsg_level != libc::SOL_SOCKET
                    || (*header).cmsg_type != libc::SCM_RIGHTS
                {
                    invalid = true
                } else {
                    let count = (size - minimum) / mem::size_of::<libc::c_int>();
                    if !(size - minimum).is_multiple_of(mem::size_of::<libc::c_int>()) {
                        invalid = true
                    }
                    for index in 0..count {
                        let fd = std::ptr::read_unaligned(
                            libc::CMSG_DATA(header).cast::<libc::c_int>().add(index),
                        );
                        if fd < 0 {
                            invalid = true
                        } else {
                            incoming.push(OwnedFd::from_raw_fd(fd));
                        }
                    }
                }
                header = libc::CMSG_NXTHDR(&message, header);
            }
        }
        if invalid
            || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
            || self.descriptors.len() + incoming.len() > 64
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid broker descriptor controls",
            ));
        }
        for fd in incoming {
            if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(io::Error::last_os_error());
            }
            self.descriptors.push_back(fd)
        }
        Ok(())
    }
}
impl io::BufRead for DescriptorReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.buffer.len() {
            self.refill()?;
        }
        Ok(&self.buffer[self.position..])
    }
    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.buffer.len());
    }
}
impl io::Read for DescriptorReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        use io::BufRead;
        if output.is_empty() {
            return Ok(0);
        }
        let available = self.fill_buf()?;
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

#[cfg(test)]
mod buffered_tests {
    use super::*;
    use std::io::{BufRead, Write};
    #[test]
    fn buffered_json_read_ahead_preserves_correlated_descriptor() {
        let (mut a, b) = UnixStream::pair().unwrap();
        let file = std::fs::File::open("/dev/null").unwrap();
        a.write_all(b"{\"response\":1}\n").unwrap();
        send(&a, 41, &file).unwrap();
        a.write_all(b"{\"response\":2}\n").unwrap();
        let mut reader = DescriptorReader::new(b);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"response\":1}\n");
        drop(reader.attachment(41).unwrap());
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "{\"response\":2}\n");
    }
}

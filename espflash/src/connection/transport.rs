//! Minimal serial operations required by the flasher, independent of OS handles.
use serialport::{ClearBuffer, Result};
use std::{
    fmt::Debug,
    io::{Read, Write},
    time::Duration,
};

/// A single-owner, synchronous byte stream with bounded IO and modem controls.
///
/// Errors use serialport's portable error type, but no TTY/COM fd is required.
/// `clear(Input)` discards only previously received data; subsequent bytes must
/// remain readable. `flush` waits for queued output or reports an error.
/// Control updates must preserve the other line; `set_dtr_rts` changes both in
/// one transport operation (USB CDC SET_CONTROL_LINE_STATE, or a TTY ioctl).
pub trait Transport: Read + Write + Debug + Send {
    /// A diagnostic address, if available.
    fn name(&self) -> Option<String>;
    /// Return the configured baud rate.
    fn baud_rate(&self) -> Result<u32>;
    /// Set the baud rate (logical for fixed-rate native USB).
    fn set_baud_rate(&mut self, baud: u32) -> Result<()>;
    /// Return the IO timeout.
    fn timeout(&self) -> Duration;
    /// Set bounded read and write timeouts.
    fn set_timeout(&mut self, timeout: Duration) -> Result<()>;
    /// Report buffered input available without blocking.
    fn bytes_to_read(&mut self) -> Result<u32>;
    /// Discard queued input or output as requested.
    fn clear(&mut self, buffer: ClearBuffer) -> Result<()>;
    /// Set DTR while preserving RTS.
    fn write_data_terminal_ready(&mut self, level: bool) -> Result<()>;
    /// Set RTS while preserving DTR.
    fn write_request_to_send(&mut self, level: bool) -> Result<()>;
    /// Set both modem lines in one transport operation.
    fn set_dtr_rts(&mut self, dtr: bool, rts: bool) -> Result<()>;
}

macro_rules! native_transport {
    ($port:ty) => {
        impl Transport for $port {
            fn name(&self) -> Option<String> {
                serialport::SerialPort::name(self)
            }
            fn baud_rate(&self) -> Result<u32> {
                serialport::SerialPort::baud_rate(self)
            }
            fn set_baud_rate(&mut self, baud: u32) -> Result<()> {
                serialport::SerialPort::set_baud_rate(self, baud)
            }
            fn timeout(&self) -> Duration {
                serialport::SerialPort::timeout(self)
            }
            fn set_timeout(&mut self, t: Duration) -> Result<()> {
                serialport::SerialPort::set_timeout(self, t)
            }
            fn bytes_to_read(&mut self) -> Result<u32> {
                serialport::SerialPort::bytes_to_read(self)
            }
            fn clear(&mut self, b: ClearBuffer) -> Result<()> {
                serialport::SerialPort::clear(self, b)
            }
            fn write_data_terminal_ready(&mut self, v: bool) -> Result<()> {
                serialport::SerialPort::write_data_terminal_ready(self, v)
            }
            fn write_request_to_send(&mut self, v: bool) -> Result<()> {
                serialport::SerialPort::write_request_to_send(self, v)
            }
            fn set_dtr_rts(&mut self, dtr: bool, rts: bool) -> Result<()> {
                native_lines(self, dtr, rts)
            }
        }
    };
}
#[cfg(unix)]
native_transport!(serialport::TTYPort);
#[cfg(windows)]
native_transport!(serialport::COMPort);

#[cfg(unix)]
fn native_lines(port: &mut serialport::TTYPort, dtr: bool, rts: bool) -> Result<()> {
    use std::os::fd::AsRawFd;
    let fd = port.as_raw_fd();
    let mut status = 0i32;
    // The native adapter alone uses TTY ioctls; USB adapters implement their own control transfer.
    if unsafe { libc::ioctl(fd, libc::TIOCMGET, &mut status) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if dtr {
        status |= libc::TIOCM_DTR;
    } else {
        status &= !libc::TIOCM_DTR;
    }
    if rts {
        status |= libc::TIOCM_RTS;
    } else {
        status &= !libc::TIOCM_RTS;
    }
    if unsafe { libc::ioctl(fd, libc::TIOCMSET, &status) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(windows)]
fn native_lines(_: &mut serialport::COMPort, _: bool, _: bool) -> Result<()> {
    Err(serialport::Error::new(
        serialport::ErrorKind::Unknown,
        "atomic modem control is unavailable on this adapter",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::connection::{
        Connection, ResetAfterOperation, ResetBeforeOperation,
        reset::{ResetStrategy, UnixTightReset},
    };
    use std::{
        io,
        sync::{Arc, Mutex},
    };
    #[derive(Debug)]
    struct Fake {
        calls: Arc<Mutex<Vec<(bool, bool)>>>,
        timeout: Duration,
        fail: bool,
    }
    impl Read for Fake {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::TimedOut.into())
        }
    }
    impl Write for Fake {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Transport for Fake {
        fn name(&self) -> Option<String> {
            None
        }
        fn baud_rate(&self) -> Result<u32> {
            Ok(115200)
        }
        fn set_baud_rate(&mut self, _: u32) -> Result<()> {
            Ok(())
        }
        fn timeout(&self) -> Duration {
            self.timeout
        }
        fn set_timeout(&mut self, t: Duration) -> Result<()> {
            self.timeout = t;
            Ok(())
        }
        fn bytes_to_read(&mut self) -> Result<u32> {
            Ok(0)
        }
        fn clear(&mut self, _: ClearBuffer) -> Result<()> {
            Ok(())
        }
        fn write_data_terminal_ready(&mut self, _: bool) -> Result<()> {
            panic!("expected atomic control")
        }
        fn write_request_to_send(&mut self, _: bool) -> Result<()> {
            panic!("expected atomic control")
        }
        fn set_dtr_rts(&mut self, d: bool, r: bool) -> Result<()> {
            if self.fail {
                return Err(serialport::Error::new(
                    serialport::ErrorKind::Unknown,
                    "disconnected",
                ));
            }
            self.calls.lock().unwrap().push((d, r));
            Ok(())
        }
    }
    #[test]
    fn custom_port_restores_timeout_on_protocol_error() {
        let port = Fake {
            calls: Default::default(),
            timeout: Duration::from_secs(3),
            fail: false,
        };
        let mut connection = Connection::new(
            Box::new(port),
            serialport::UsbPortInfo {
                vid: 0x303a,
                pid: 0x1001,
                serial_number: None,
                manufacturer: None,
                product: None,
            },
            ResetAfterOperation::NoResetNoStub,
            ResetBeforeOperation::NoReset,
            115200,
        );
        let result: std::result::Result<(), crate::Error> = connection
            .with_timeout(Duration::from_millis(1), |_| {
                Err(crate::Error::IncorrectResponse)
            });
        assert!(result.is_err());
        assert_eq!(connection.into_serial().timeout(), Duration::from_secs(3));
    }
    #[test]
    fn reset_delegates_atomic_lines_and_propagates_disconnect() {
        let calls = Arc::default();
        let mut port: Box<dyn Transport> = Box::new(Fake {
            calls: Arc::clone(&calls),
            timeout: Duration::from_secs(1),
            fail: false,
        });
        let strategy = UnixTightReset::new(false);
        strategy.set_dtr_rts(&mut port, true, false).unwrap();
        assert_eq!(*calls.lock().unwrap(), vec![(true, false)]);
        let mut failed: Box<dyn Transport> = Box::new(Fake {
            calls,
            timeout: Duration::from_secs(1),
            fail: true,
        });
        assert!(strategy.set_dtr_rts(&mut failed, false, true).is_err());
    }
}

# Injectable transport fork

This branch is based on upstream espflash v4.5.0. It adds an opt-in
`custom-transport` library feature for callers with an already-authorized USB
connection, including Android USB Host.

With this feature, `connection::Port` is `Box<dyn connection::Transport>`.
Implement the bounded byte-stream and modem-control methods, then pass the owned
transport to `Connection::new`. Bootloader commands, stubs, image handling and
verification are unchanged. Without the feature, the original TTYPort/COMPort
API remains unchanged; native ports also implement the new trait.

The caller retains responsibility for USB permission, device identity, endpoint
selection, driver implementation, lifecycle and exclusive ownership. A USB device
fd is not a TTY fd. Do not apply TTY ioctls to it. Atomic DTR/RTS changes must be
implemented by the transport (for example, a single CDC control transfer).

The Android consumer is idf-remote. It supports native Espressif USB Serial/JTAG;
this fork itself does not add Android permissions, JNI, USB drivers, or support
for additional USB-UART bridges.

Validation:

```sh
cargo test -p espflash --lib --no-default-features --features custom-transport
cargo check -p espflash
cargo check -p espflash --features custom-transport
```

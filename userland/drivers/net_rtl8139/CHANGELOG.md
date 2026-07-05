# Changelog

All notable changes to `net_rtl8139` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] - 2026-07-05

### Fixed

- `send()` always reused transmit descriptor 0, kicking off every
  transmission by rewriting `TSAD0`/`TSD0` again -- which silently never
  completes a *second* transmission: both real RTL8139 hardware and
  QEMU's emulation of it track an internal "next expected descriptor"
  pointer that advances after each completion, and rewriting the one
  that was just used instead of the next one in sequence is never
  picked up. Never caught before now because nothing in this project
  had ever called `send()` more than once in a single boot until
  `netstack` did. Now rotates through all four descriptors the hardware
  actually has, in order, one frame in flight at a time as before.

## [0.1.0] - 2026-07-04

Initial release.

### Added

- A minimal RTL8139 Fast Ethernet driver: hardware init (reset, receive
  ring setup, RX/TX enable), one frame at a time transmit via a single
  descriptor, and interrupt-driven receive-ring parsing. Every hardware
  handshake this driver polls for (reset completion, transmit-descriptor
  ownership) is bounded, not an indefinite busy-wait, so a stuck card
  can't hang the driver's whole service loop forever.
- Registers as `"net"` and serves raw Ethernet frames in and out over a
  client-supplied shared-memory grant (`NIC_OP_SET_BUFFER`/`SET_REPLY`/
  `GET_MAC`/`SEND`/`RECV` -- see `userland/libpcern`). No ARP, no IP,
  nothing above the Ethernet-frame layer.
- Only one client at a time, and on the receive side only the single
  most recently received frame is ever held for a client to claim (not
  a queue) -- the same scope-narrowing precedent as every other driver
  in this project.

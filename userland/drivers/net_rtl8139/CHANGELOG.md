# Changelog

All notable changes to `net_rtl8139` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-04

Initial release.

### Added

- A minimal RTL8139 Fast Ethernet driver: hardware init (reset, receive
  ring setup, RX/TX enable), one frame at a time transmit via a single
  descriptor, and interrupt-driven receive-ring parsing.
- Registers as `"net"` and serves raw Ethernet frames in and out over a
  client-supplied shared-memory grant (`NIC_OP_SET_BUFFER`/`SET_REPLY`/
  `GET_MAC`/`SEND`/`RECV` -- see `userland/libpcern`). No ARP, no IP,
  nothing above the Ethernet-frame layer.
- Only one client at a time, and on the receive side only the single
  most recently received frame is ever held for a client to claim (not
  a queue) -- the same scope-narrowing precedent as every other driver
  in this project.

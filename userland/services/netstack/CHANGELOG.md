# Changelog

All notable changes to `netstack` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-04

Initial release.

### Added

- A minimal ARP + IPv4 + ICMP responder: claims a static IP address,
  answers ARP requests for it, and replies to ICMP echo requests
  (ping) addressed to it, over raw Ethernet frames served by
  `net_rtl8139`. No outbound connections, no DHCP, no ARP cache, no IP
  forwarding.

//! Serving beyond this machine: which interface to bind, which `Host` names to answer to, and
//! which address to tell a person to type.
//!
//! **Loopback is still the default, and leaving it is an explicit opt-in** (`--bind`). The server
//! has no authentication, and the settings API writes project files, so the decision to listen on
//! a network is the operator's, for a network they trust (a closed one: this is not built to face
//! the internet and nothing here makes it safe to).
//!
//! **The `Host` check stays meaningful off loopback.** `guard_host` in `main.rs` exists to stop DNS
//! rebinding: a hostile page on `evil.example` is re-pointed at this server, the browser sends
//! `Host: evil.example`, and the request is same-origin so CORS never decides. Serving a network
//! must not switch that off, so a non-loopback bind widens the allowance only to what a rebinding
//! attack cannot present: an **IP literal** (it cannot be re-pointed), this machine's own
//! hostname, and any name the operator lists. A `Host` naming some other domain is still refused.
//! A pyRevit pusher says `Host: 192.168.1.20:5151`, which is why the literal is allowed.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

/// Strip the `:port` suffix and any IPv6 brackets from a `Host` header value.
/// IPv6 literals are bracketed (`[::1]:5151`), so the last colon is only a
/// port separator when it falls outside the brackets.
pub fn host_name(value: &str) -> &str {
    let bare = if let Some(end) = value.find(']') {
        &value[..end + 1]
    } else if value.matches(':').count() == 1 {
        value.split(':').next().unwrap_or(value)
    } else {
        // A bare IPv6 literal with no brackets and no port.
        value
    };
    bare.trim_start_matches('[').trim_end_matches(']')
}

const LOOPBACK_NAMES: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// The `Host` names the server answers to.
#[derive(Debug, Clone)]
pub struct HostPolicy {
    /// Serving a network: any IP literal is allowed.
    network: bool,
    /// Lower-case names allowed besides loopback: this machine's, and the operator's.
    names: BTreeSet<String>,
}

impl HostPolicy {
    /// Loopback names only: what the server has always answered to.
    pub fn loopback_only() -> Self {
        HostPolicy { network: false, names: BTreeSet::new() }
    }

    /// The policy for `bind`, this machine's `hostname` (when it has one) and the operator's
    /// `extra` names. A loopback bind keeps the loopback-only rule for IP literals, though the
    /// listed names are still allowed (reached through a tunnel, say).
    pub fn for_bind(bind: IpAddr, hostname: Option<&str>, extra: &[String]) -> Self {
        let mut names: BTreeSet<String> = extra
            .iter()
            .map(|n| host_name(n.trim()).to_ascii_lowercase())
            .filter(|n| !n.is_empty())
            .collect();
        if !bind.is_loopback()
            && let Some(h) = hostname.map(|h| h.trim().to_ascii_lowercase()).filter(|h| !h.is_empty())
        {
            // `Mac-Studio`, and `Mac-Studio.local` which is how another machine usually spells it.
            let short = h.split('.').next().unwrap_or(&h).to_string();
            names.insert(format!("{short}.local"));
            names.insert(short);
            names.insert(h);
        }
        HostPolicy { network: !bind.is_loopback(), names }
    }

    /// Whether a request naming `host` (a `Host` header value, port and all) is answered.
    pub fn allows(&self, host: &str) -> bool {
        let name = host_name(host).to_ascii_lowercase();
        if LOOPBACK_NAMES.contains(&name.as_str()) || self.names.contains(&name) {
            return true;
        }
        self.network && name.trim_matches(['[', ']']).parse::<IpAddr>().is_ok()
    }
}

/// Whether binding `addr` leaves this machine.
pub fn serves_network(addr: IpAddr) -> bool {
    !addr.is_loopback()
}

/// This machine's name as the operating system has it, or `None`. Asked of the `hostname` program
/// rather than a crate: it exists on macOS, Linux and Windows, and this runs once at startup.
pub fn machine_hostname() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    let name = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// The IPv4 address this machine uses to reach the rest of the network. A UDP "connect" picks the
/// route without sending a packet, so it needs no interface-listing crate. `None` when there is no
/// route (no network at all).
fn primary_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// The addresses to tell a person to type for a server bound to `bind`, for the startup banner.
/// A specific bind is itself; a wildcard bind is this machine's primary address and, when it has
/// a name, `<name>.local`.
pub fn reachable_urls(bind: SocketAddr, hostname: Option<&str>) -> Vec<String> {
    let port = bind.port();
    if bind.ip().is_loopback() {
        return vec![format!("http://127.0.0.1:{port}")];
    }
    if !bind.ip().is_unspecified() {
        let ip = bind.ip();
        return vec![match ip {
            IpAddr::V6(_) => format!("http://[{ip}]:{port}"),
            IpAddr::V4(_) => format!("http://{ip}:{port}"),
        }];
    }
    let mut urls = Vec::new();
    if let Some(ip) = primary_ipv4() {
        urls.push(format!("http://{ip}:{port}"));
    }
    if let Some(h) = hostname {
        let short = h.split('.').next().unwrap_or(h).to_ascii_lowercase();
        urls.push(format!("http://{short}.local:{port}"));
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// The port split has to survive IPv6 literals, where colons are part of
    /// the address rather than a port separator.
    #[test]
    fn test_host_name_strips_port_not_address() {
        assert_eq!(host_name("localhost:5151"), "localhost");
        assert_eq!(host_name("127.0.0.1"), "127.0.0.1");
        assert_eq!(host_name("[::1]:5151"), "::1");
        assert_eq!(host_name("[::1]"), "::1");
        assert_eq!(host_name("::1"), "::1");
    }

    #[test]
    fn test_loopback_only_answers_loopback_names_and_nothing_else() {
        let p = HostPolicy::loopback_only();
        for ok in ["localhost", "localhost:5151", "127.0.0.1:5151", "[::1]:5151"] {
            assert!(p.allows(ok), "{ok}");
        }
        for no in ["192.168.1.20:5151", "evil.example", "mac-studio.local", "10.0.0.5"] {
            assert!(!p.allows(no), "{no}");
        }
    }

    #[test]
    fn test_a_network_bind_allows_ip_literals_but_still_refuses_other_domains() {
        let p = HostPolicy::for_bind(ip("0.0.0.0"), Some("Mac-Studio.lan"), &[]);
        for ok in ["192.168.1.20:5151", "10.0.0.5", "[fe80::1]:5151", "[2001:db8::1]"] {
            assert!(p.allows(ok), "an IP literal cannot be re-pointed by a rebinding page: {ok}");
        }
        for no in [
            "evil.example",
            "evil.example:5151",
            "192.168.1.20.evil.example",
            "roommate.attacker.test",
        ] {
            assert!(!p.allows(no), "a domain this machine does not own must be refused: {no}");
        }
    }

    #[test]
    fn test_a_network_bind_answers_to_this_machines_names_in_any_case() {
        let p = HostPolicy::for_bind(ip("192.168.1.20"), Some("Mac-Studio.lan"), &[]);
        for ok in ["mac-studio", "MAC-STUDIO:5151", "Mac-Studio.local", "mac-studio.lan"] {
            assert!(p.allows(ok), "{ok}");
        }
    }

    #[test]
    fn test_listed_names_are_allowed_and_a_loopback_bind_keeps_literals_out() {
        let extra = vec!["roommate.studio.internal".to_string(), " Plans:5151 ".to_string()];
        let net = HostPolicy::for_bind(ip("0.0.0.0"), None, &extra);
        assert!(net.allows("roommate.studio.internal:5151"));
        assert!(net.allows("plans"));
        let local = HostPolicy::for_bind(ip("127.0.0.1"), Some("Mac-Studio"), &extra);
        assert!(local.allows("plans"), "a listed name is the operator's word");
        assert!(!local.allows("192.168.1.20"), "a loopback bind is not reachable by address anyway");
        assert!(!local.allows("mac-studio"), "this machine's name is only added when serving a network");
    }

    #[test]
    fn test_what_to_type_for_each_kind_of_bind() {
        assert_eq!(
            reachable_urls("127.0.0.1:5151".parse().unwrap(), Some("m")),
            vec!["http://127.0.0.1:5151"]
        );
        assert_eq!(
            reachable_urls("192.168.1.20:6000".parse().unwrap(), None),
            vec!["http://192.168.1.20:6000"]
        );
        assert_eq!(reachable_urls("[fe80::1]:6000".parse().unwrap(), None), vec!["http://[fe80::1]:6000"]);
        let wild = reachable_urls("0.0.0.0:5151".parse().unwrap(), Some("Mac-Studio.lan"));
        assert!(wild.contains(&"http://mac-studio.local:5151".to_string()), "{wild:?}");
        assert!(wild.iter().all(|u| u.ends_with(":5151")));
    }

    #[test]
    fn test_only_a_non_loopback_bind_serves_a_network() {
        assert!(!serves_network(ip("127.0.0.1")));
        assert!(!serves_network(ip("::1")));
        assert!(serves_network(ip("0.0.0.0")));
        assert!(serves_network(ip("192.168.1.20")));
    }
}

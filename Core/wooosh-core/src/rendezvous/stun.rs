//! Just enough STUN (RFC 8489) for two jobs: asking a public STUN server for
//! this socket's mapped address, and the authenticated punch probes peers send
//! each other (PROTOCOL.md §9.5). Probes are Binding requests carrying a
//! Wooosh MAC attribute, so a NAT or firewall that inspects STUN sees STUN.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

pub const MAGIC_COOKIE: u32 = 0x2112_A442;
pub const BINDING_REQUEST: u16 = 0x0001;
pub const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
/// Comprehension-optional range, so a strict STUN server ignores it.
pub const ATTR_WOOOSH_AUTH: u16 = 0xC057;

pub type TxId = [u8; 12];

/// RFC 9443 demultiplexing: STUN's first byte is 0-3, QUIC's always has the
/// fixed bit (0x40) set.
pub fn looks_like_stun(packet: &[u8]) -> bool {
    packet.len() >= 20
        && packet[0] < 4
        && u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]) == MAGIC_COOKIE
}

pub struct Message<'a> {
    pub kind: u16,
    pub txid: TxId,
    attrs: &'a [u8],
}

impl<'a> Message<'a> {
    pub fn parse(packet: &'a [u8]) -> Option<Self> {
        if !looks_like_stun(packet) {
            return None;
        }
        let kind = u16::from_be_bytes([packet[0], packet[1]]);
        let len = u16::from_be_bytes([packet[2], packet[3]]) as usize;
        if packet.len() < 20 + len {
            return None;
        }
        let mut txid = [0u8; 12];
        txid.copy_from_slice(&packet[8..20]);
        Some(Message { kind, txid, attrs: &packet[20..20 + len] })
    }

    pub fn attr(&self, kind: u16) -> Option<&'a [u8]> {
        let mut p = 0;
        while p + 4 <= self.attrs.len() {
            let t = u16::from_be_bytes([self.attrs[p], self.attrs[p + 1]]);
            let l = u16::from_be_bytes([self.attrs[p + 2], self.attrs[p + 3]]) as usize;
            let v = self.attrs.get(p + 4..p + 4 + l)?;
            if t == kind {
                return Some(v);
            }
            p += 4 + ((l + 3) & !3);
        }
        None
    }

    pub fn mapped_address(&self) -> Option<SocketAddr> {
        if let Some(v) = self.attr(ATTR_XOR_MAPPED_ADDRESS) {
            return decode_address(v, Some(&self.txid));
        }
        decode_address(self.attr(ATTR_MAPPED_ADDRESS)?, None)
    }
}

fn decode_address(v: &[u8], xor_txid: Option<&TxId>) -> Option<SocketAddr> {
    if v.len() < 4 {
        return None;
    }
    let cookie = MAGIC_COOKIE.to_be_bytes();
    let mut port = u16::from_be_bytes([v[2], v[3]]);
    if xor_txid.is_some() {
        port ^= (MAGIC_COOKIE >> 16) as u16;
    }
    let ip = match v[1] {
        1 if v.len() >= 8 => {
            let mut b = [v[4], v[5], v[6], v[7]];
            if xor_txid.is_some() {
                for i in 0..4 {
                    b[i] ^= cookie[i];
                }
            }
            IpAddr::V4(Ipv4Addr::from(b))
        }
        2 if v.len() >= 20 => {
            let mut b = [0u8; 16];
            b.copy_from_slice(&v[4..20]);
            if let Some(txid) = xor_txid {
                let mut key = [0u8; 16];
                key[..4].copy_from_slice(&cookie);
                key[4..].copy_from_slice(txid);
                for i in 0..16 {
                    b[i] ^= key[i];
                }
            }
            IpAddr::V6(Ipv6Addr::from(b))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

fn encode_xor_address(addr: SocketAddr, txid: &TxId) -> Vec<u8> {
    let cookie = MAGIC_COOKIE.to_be_bytes();
    let mut v = vec![0u8];
    let port = addr.port() ^ (MAGIC_COOKIE >> 16) as u16;
    match addr.ip() {
        IpAddr::V4(ip) => {
            v.push(1);
            v.extend_from_slice(&port.to_be_bytes());
            v.extend(ip.octets().iter().zip(cookie).map(|(a, b)| a ^ b));
        }
        IpAddr::V6(ip) => {
            v.push(2);
            v.extend_from_slice(&port.to_be_bytes());
            let mut key = [0u8; 16];
            key[..4].copy_from_slice(&cookie);
            key[4..].copy_from_slice(txid);
            v.extend(ip.octets().iter().zip(key).map(|(a, b)| a ^ b));
        }
    }
    v
}

pub fn encode(kind: u16, txid: &TxId, attrs: &[(u16, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (t, v) in attrs {
        body.extend_from_slice(&t.to_be_bytes());
        body.extend_from_slice(&(v.len() as u16).to_be_bytes());
        body.extend_from_slice(v);
        body.resize((body.len() + 3) & !3, 0);
    }
    let mut out = Vec::with_capacity(20 + body.len());
    out.extend_from_slice(&kind.to_be_bytes());
    out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
    out.extend_from_slice(txid);
    out.extend_from_slice(&body);
    out
}

pub fn binding_request(txid: &TxId, auth: Option<&[u8]>) -> Vec<u8> {
    match auth {
        Some(a) => encode(BINDING_REQUEST, txid, &[(ATTR_WOOOSH_AUTH, a)]),
        None => encode(BINDING_REQUEST, txid, &[]),
    }
}

pub fn binding_success(txid: &TxId, observed: SocketAddr, auth: &[u8]) -> Vec<u8> {
    let addr = encode_xor_address(observed, txid);
    encode(BINDING_SUCCESS, txid, &[(ATTR_XOR_MAPPED_ADDRESS, &addr), (ATTR_WOOOSH_AUTH, auth)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_mapped_address_roundtrips_both_families() {
        let txid = [7u8; 12];
        for addr in ["203.0.113.9:41000", "[2001:db8::5]:443"] {
            let a: SocketAddr = addr.parse().unwrap();
            let pkt = binding_success(&txid, a, &[1; 16]);
            let m = Message::parse(&pkt).unwrap();
            assert_eq!(m.kind, BINDING_SUCCESS);
            assert_eq!(m.mapped_address(), Some(a));
            assert_eq!(m.attr(ATTR_WOOOSH_AUTH), Some(&[1u8; 16][..]));
        }
    }

    #[test]
    fn rfc5769_ipv4_response_vector() {
        // RFC 5769 §2.2 sample IPv4 response; mapped address 192.0.2.1:32853.
        let hexstr = concat!(
            "0101003c", "2112a442", "b7e7a701bc34d686fa87dfae", // header
            "8022000b", "7465737420766563746f7220",             // SOFTWARE "test vector "
            "00200008", "0001a147", "e112a643",                 // XOR-MAPPED-ADDRESS
            "00080014", "2b91f599fd9e90c38c7489f92af9ba53f06be7d7", // MESSAGE-INTEGRITY
            "80280004", "c07d4c96",                             // FINGERPRINT
        );
        let pkt = hex::decode(hexstr).unwrap();
        let m = Message::parse(&pkt).unwrap();
        assert_eq!(m.mapped_address(), Some("192.0.2.1:32853".parse().unwrap()));
    }

    #[test]
    fn demultiplexes_quic_from_stun() {
        assert!(looks_like_stun(&binding_request(&[0; 12], None)));
        let mut quic_short = vec![0x40u8; 32];
        quic_short[0] = 0x41;
        assert!(!looks_like_stun(&quic_short));
        let mut quic_long = vec![0u8; 1200];
        quic_long[0] = 0xc3;
        assert!(!looks_like_stun(&quic_long));
    }
}

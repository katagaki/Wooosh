//! Control protocol messages + framing (PROTOCOL.md §5).
//!
//! CBOR maps with string keys and an integer `t` type tag, length-prefixed
//! with u32 big-endian on the control stream. Unknown map keys are ignored
//! (forward compatibility); unknown `t` values yield `Msg::Unknown`.
//!
//! PROTOCOL.md gives HELLO no explicit tag and reserves t:16..19 for PAIR_*
//! without assigning them individually, so these are wire-compatibility
//! commitments, not free choices: HELLO=0, PAIR_REQUEST=16, PAIR_ACCEPT=17,
//! PAIR_CONFIRM=18, PAIR_REJECT=19, ERR_UNSUPPORTED=254, BYE=255.

use ciborium::Value;

pub const PROTOCOL_VERSION: u64 = 1;
/// Sanity cap for control frames (a 10k-file manifest fits comfortably).
pub const MAX_FRAME: u32 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct FileMeta {
    pub fid: u32,
    pub name: String,
    pub rel_path: Option<String>,
    pub size: u64,
    pub mime: String,
    pub b3: [u8; 32],
    pub mtime: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeHave {
    pub fid: u32,
    pub verified_off: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Hello { v: u64, device_id: Vec<u8>, dn: String, dt: String, caps: Vec<String> },
    Offer { tid: [u8; 16], files: Vec<FileMeta>, total: u64, note: Option<String> },
    Decision { tid: [u8; 16], accept: Vec<u32> },
    ResumeQ { tid: [u8; 16] },
    ResumeA { tid: [u8; 16], have: Vec<ResumeHave> },
    Done { tid: [u8; 16], fid: u32, ok: bool, err: Option<String> },
    Cancel { tid: [u8; 16], fid: Option<u32> },
    PairRequest { token: Option<Vec<u8>> },
    PairAccept,
    PairConfirm,
    PairReject,
    ErrUnsupported { t: u64 },
    Bye,
    /// Unknown `t` — must be answered with ErrUnsupported, connection stays up.
    Unknown { t: u64 },
}

// ---------- encoding helpers ----------

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(entries.into_iter().map(|(k, v)| (Value::Text(k.into()), v)).collect())
}

fn b(bytes: &[u8]) -> Value {
    Value::Bytes(bytes.to_vec())
}

fn u(v: u64) -> Value {
    Value::Integer(v.into())
}

impl Msg {
    pub fn type_tag(&self) -> u64 {
        match self {
            Msg::Hello { .. } => 0,
            Msg::Offer { .. } => 1,
            Msg::Decision { .. } => 2,
            Msg::ResumeQ { .. } => 3,
            Msg::ResumeA { .. } => 4,
            Msg::Done { .. } => 5,
            Msg::Cancel { .. } => 6,
            Msg::PairRequest { .. } => 16,
            Msg::PairAccept => 17,
            Msg::PairConfirm => 18,
            Msg::PairReject => 19,
            Msg::ErrUnsupported { .. } => 254,
            Msg::Bye => 255,
            Msg::Unknown { t } => *t,
        }
    }

    pub fn to_value(&self) -> Value {
        let t = u(self.type_tag());
        match self {
            Msg::Hello { v, device_id, dn, dt, caps } => map(vec![
                ("t", t),
                ("v", u(*v)),
                ("device_id", b(device_id)),
                ("dn", Value::Text(dn.clone())),
                ("dt", Value::Text(dt.clone())),
                ("caps", Value::Array(caps.iter().map(|c| Value::Text(c.clone())).collect())),
            ]),
            Msg::Offer { tid, files, total, note } => {
                let mut e = vec![
                    ("t", t),
                    ("tid", b(tid)),
                    (
                        "files",
                        Value::Array(
                            files
                                .iter()
                                .map(|f| {
                                    let mut fe = vec![
                                        ("fid", u(f.fid as u64)),
                                        ("name", Value::Text(f.name.clone())),
                                    ];
                                    if let Some(rp) = &f.rel_path {
                                        fe.push(("rel_path", Value::Text(rp.clone())));
                                    }
                                    fe.push(("size", u(f.size)));
                                    fe.push(("mime", Value::Text(f.mime.clone())));
                                    fe.push(("b3", b(&f.b3)));
                                    fe.push(("mtime", u(f.mtime)));
                                    map(fe)
                                })
                                .collect(),
                        ),
                    ),
                    ("total", u(*total)),
                ];
                if let Some(n) = note {
                    e.push(("note", Value::Text(n.clone())));
                }
                map(e)
            }
            Msg::Decision { tid, accept } => map(vec![
                ("t", t),
                ("tid", b(tid)),
                ("accept", Value::Array(accept.iter().map(|f| u(*f as u64)).collect())),
            ]),
            Msg::ResumeQ { tid } => map(vec![("t", t), ("tid", b(tid))]),
            Msg::ResumeA { tid, have } => map(vec![
                ("t", t),
                ("tid", b(tid)),
                (
                    "have",
                    Value::Array(
                        have.iter()
                            .map(|h| {
                                map(vec![
                                    ("fid", u(h.fid as u64)),
                                    ("verified_off", u(h.verified_off)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
            Msg::Done { tid, fid, ok, err } => {
                let mut e = vec![
                    ("t", t),
                    ("tid", b(tid)),
                    ("fid", u(*fid as u64)),
                    ("ok", Value::Bool(*ok)),
                ];
                if let Some(er) = err {
                    e.push(("err", Value::Text(er.clone())));
                }
                map(e)
            }
            Msg::Cancel { tid, fid } => {
                let mut e = vec![("t", t), ("tid", b(tid))];
                if let Some(f) = fid {
                    e.push(("fid", u(*f as u64)));
                }
                map(e)
            }
            Msg::PairRequest { token } => {
                let mut e = vec![("t", t)];
                if let Some(tok) = token {
                    e.push(("tok", b(tok)));
                }
                map(e)
            }
            Msg::PairAccept | Msg::PairConfirm | Msg::PairReject | Msg::Bye => map(vec![("t", t)]),
            Msg::ErrUnsupported { t: bad } => map(vec![("t", t), ("bad_t", u(*bad))]),
            Msg::Unknown { .. } => map(vec![("t", t)]),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&self.to_value(), &mut buf).expect("cbor encode");
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Msg, String> {
        let v: Value = ciborium::de::from_reader(data).map_err(|e| format!("cbor: {e}"))?;
        Msg::from_value(&v)
    }

    pub fn from_value(v: &Value) -> Result<Msg, String> {
        let m = as_map(v)?;
        let t = get_u64(m, "t").ok_or("missing t")?;
        Ok(match t {
            0 => Msg::Hello {
                v: get_u64(m, "v").ok_or("hello: missing v")?,
                device_id: get_bytes(m, "device_id").unwrap_or_default(),
                dn: get_str(m, "dn").unwrap_or_default(),
                dt: get_str(m, "dt").unwrap_or_default(),
                caps: get_str_array(m, "caps"),
            },
            1 => {
                let files_v = get(m, "files").ok_or("offer: missing files")?;
                let arr = files_v.as_array().ok_or("offer: files not array")?;
                let mut files = Vec::with_capacity(arr.len());
                for fv in arr {
                    let fm = as_map(fv)?;
                    files.push(FileMeta {
                        fid: get_u64(fm, "fid").ok_or("filemeta: fid")? as u32,
                        name: get_str(fm, "name").ok_or("filemeta: name")?,
                        rel_path: get_str(fm, "rel_path"),
                        size: get_u64(fm, "size").ok_or("filemeta: size")?,
                        mime: get_str(fm, "mime").unwrap_or_default(),
                        b3: get_bytes32(fm, "b3").ok_or("filemeta: b3")?,
                        mtime: get_u64(fm, "mtime").unwrap_or(0),
                    });
                }
                Msg::Offer {
                    tid: get_tid(m)?,
                    files,
                    total: get_u64(m, "total").unwrap_or(0),
                    note: get_str(m, "note"),
                }
            }
            2 => Msg::Decision {
                tid: get_tid(m)?,
                accept: get(m, "accept")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(val_u64).map(|x| x as u32).collect())
                    .unwrap_or_default(),
            },
            3 => Msg::ResumeQ { tid: get_tid(m)? },
            4 => {
                let mut have = Vec::new();
                if let Some(arr) = get(m, "have").and_then(|v| v.as_array()) {
                    for hv in arr {
                        let hm = as_map(hv)?;
                        have.push(ResumeHave {
                            fid: get_u64(hm, "fid").ok_or("resume_a: fid")? as u32,
                            verified_off: get_u64(hm, "verified_off").unwrap_or(0),
                        });
                    }
                }
                Msg::ResumeA { tid: get_tid(m)?, have }
            }
            5 => Msg::Done {
                tid: get_tid(m)?,
                fid: get_u64(m, "fid").ok_or("done: fid")? as u32,
                ok: get(m, "ok").and_then(|v| v.as_bool()).unwrap_or(false),
                err: get_str(m, "err"),
            },
            6 => Msg::Cancel { tid: get_tid(m)?, fid: get_u64(m, "fid").map(|f| f as u32) },
            16 => Msg::PairRequest { token: get_bytes(m, "tok") },
            17 => Msg::PairAccept,
            18 => Msg::PairConfirm,
            19 => Msg::PairReject,
            254 => Msg::ErrUnsupported { t: get_u64(m, "bad_t").unwrap_or(0) },
            255 => Msg::Bye,
            other => Msg::Unknown { t: other },
        })
    }
}

// ---------- file-stream header (PROTOCOL.md §6) ----------

#[derive(Debug, Clone, PartialEq)]
pub struct StreamHeader {
    pub tid: [u8; 16],
    pub fid: u32,
    pub off: u64,
}

impl StreamHeader {
    pub fn encode(&self) -> Vec<u8> {
        let v = map(vec![("tid", b(&self.tid)), ("fid", u(self.fid as u64)), ("off", u(self.off))]);
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&v, &mut buf).expect("cbor encode");
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, String> {
        let v: Value = ciborium::de::from_reader(data).map_err(|e| format!("cbor: {e}"))?;
        let m = as_map(&v)?;
        Ok(StreamHeader {
            tid: get_tid(m)?,
            fid: get_u64(m, "fid").ok_or("header: fid")? as u32,
            off: get_u64(m, "off").unwrap_or(0),
        })
    }
}

// ---------- framing ----------

/// Frame = u32 BE length + CBOR body.
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    out
}

// ---------- map helpers ----------

type CborMap = [(Value, Value)];

fn as_map(v: &Value) -> Result<&CborMap, String> {
    v.as_map().map(|m| m.as_slice()).ok_or_else(|| "expected cbor map".to_string())
}

fn get<'a>(m: &'a CborMap, key: &str) -> Option<&'a Value> {
    m.iter().find(|(k, _)| k.as_text() == Some(key)).map(|(_, v)| v)
}

fn val_u64(v: &Value) -> Option<u64> {
    v.as_integer().and_then(|i| u64::try_from(i).ok())
}

fn get_u64(m: &CborMap, key: &str) -> Option<u64> {
    get(m, key).and_then(val_u64)
}

fn get_str(m: &CborMap, key: &str) -> Option<String> {
    get(m, key).and_then(|v| v.as_text()).map(|s| s.to_string())
}

fn get_bytes(m: &CborMap, key: &str) -> Option<Vec<u8>> {
    get(m, key).and_then(|v| v.as_bytes()).cloned()
}

fn get_bytes32(m: &CborMap, key: &str) -> Option<[u8; 32]> {
    get_bytes(m, key).and_then(|b| b.try_into().ok())
}

fn get_str_array(m: &CborMap, key: &str) -> Vec<String> {
    get(m, key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_text().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn get_tid(m: &CborMap) -> Result<[u8; 16], String> {
    get_bytes(m, "tid").and_then(|b| b.try_into().ok()).ok_or_else(|| "bad tid".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(msg: Msg) {
        let enc = msg.encode();
        let dec = Msg::decode(&enc).unwrap();
        assert_eq!(msg, dec);
    }

    #[test]
    fn roundtrip_all_messages() {
        roundtrip(Msg::Hello {
            v: 1,
            device_id: vec![1; 16],
            dn: "Phone".into(),
            dt: "phone".into(),
            caps: vec!["zstd".into()],
        });
        roundtrip(Msg::Offer {
            tid: [9; 16],
            files: vec![FileMeta {
                fid: 1,
                name: "a.txt".into(),
                rel_path: Some("d/a.txt".into()),
                size: 42,
                mime: "text/plain".into(),
                b3: [3; 32],
                mtime: 1_700_000_000,
            }],
            total: 42,
            note: Some("hi".into()),
        });
        roundtrip(Msg::Decision { tid: [1; 16], accept: vec![1, 2, 3] });
        roundtrip(Msg::ResumeQ { tid: [2; 16] });
        roundtrip(Msg::ResumeA {
            tid: [2; 16],
            have: vec![ResumeHave { fid: 1, verified_off: 1024 }],
        });
        roundtrip(Msg::Done { tid: [1; 16], fid: 7, ok: false, err: Some("HASH_MISMATCH".into()) });
        roundtrip(Msg::Cancel { tid: [1; 16], fid: None });
        roundtrip(Msg::Cancel { tid: [1; 16], fid: Some(3) });
        roundtrip(Msg::PairRequest { token: Some(vec![5; 32]) });
        roundtrip(Msg::PairRequest { token: None });
        roundtrip(Msg::PairAccept);
        roundtrip(Msg::PairConfirm);
        roundtrip(Msg::PairReject);
        roundtrip(Msg::ErrUnsupported { t: 42 });
        roundtrip(Msg::Bye);
    }

    #[test]
    fn unknown_tag_is_preserved() {
        let enc = Msg::Unknown { t: 99 }.encode();
        assert_eq!(Msg::decode(&enc).unwrap(), Msg::Unknown { t: 99 });
    }

    #[test]
    fn unknown_keys_ignored() {
        // A BYE with an extra key must still parse (forward compat).
        let v = Value::Map(vec![
            (Value::Text("t".into()), Value::Integer(255.into())),
            (Value::Text("future".into()), Value::Text("stuff".into())),
        ]);
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&v, &mut buf).unwrap();
        assert_eq!(Msg::decode(&buf).unwrap(), Msg::Bye);
    }

    #[test]
    fn stream_header_roundtrip() {
        let h = StreamHeader { tid: [8; 16], fid: 12, off: 1 << 33 };
        let enc = h.encode();
        assert_eq!(StreamHeader::decode(&enc).unwrap(), h);
    }

    #[test]
    fn framing_is_u32_be() {
        let f = frame(&[0xAA, 0xBB]);
        assert_eq!(f, vec![0, 0, 0, 2, 0xAA, 0xBB]);
    }
}

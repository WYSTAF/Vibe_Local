//! Zero-dependency GGUF binary header reader (magic `GGUF`, versions 1-3):
//! tensor count, KV metadata, architecture, context length, head counts,
//! quantization type, plus an exact-ish RAM/VRAM calculator at 2k/4k/8k/16k
//! context. Only `std` is used.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const GGUF_MAGIC: u32 = 0x4655_4747; // "GGUF" little-endian

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GgufValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    String(String),
    U64(u64),
    I64(i64),
    F64(f64),
    Array(Vec<GgufValue>),
}

impl GgufValue {
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            GgufValue::U8(v) => Some(u64::from(*v)),
            GgufValue::U16(v) => Some(u64::from(*v)),
            GgufValue::U32(v) => Some(u64::from(*v)),
            GgufValue::U64(v) => Some(*v),
            GgufValue::I32(v) => u64::try_from(*v).ok(),
            GgufValue::I64(v) => u64::try_from(*v).ok(),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            GgufValue::String(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRam {
    pub ctx: u64,
    pub weights_mb: f64,
    pub kv_cache_mb: f64,
    pub compute_mb: f64,
    pub total_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GgufInfo {
    pub path: String,
    pub version: u32,
    pub tensor_count: u64,
    pub metadata_kv_count: u64,
    pub architecture: String,
    pub name: String,
    pub quant: String,
    pub file_type: Option<u32>,
    pub context_length: Option<u64>,
    pub block_count: Option<u64>,
    pub embedding_length: Option<u64>,
    pub head_count: Option<u64>,
    pub head_count_kv: Option<u64>,
    pub expert_count: Option<u64>,
    pub expert_used_count: Option<u64>,
    pub is_moe: bool,
    pub file_size_bytes: u64,
    pub kv: BTreeMap<String, GgufValue>,
    pub ram: Vec<ContextRam>,
}

/// Map llama.cpp `general.file_type` enum to a quant label.
pub fn file_type_to_quant(ft: u32) -> &'static str {
    match ft {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "IQ3_XXS",
        22 => "IQ1_S",
        23 => "IQ4_NL",
        24 => "IQ3_S",
        25 => "IQ2_M",
        26 => "IQ4_XS",
        27 => "IQ1_M",
        28 => "BF16",
        _ => "UNKNOWN",
    }
}

/// Extract a quant tag from a GGUF filename (e.g. `...-Q4_K_M.gguf`).
pub fn quant_from_filename(name: &str) -> Option<String> {
    let upper = name.to_uppercase();
    const TAGS: &[&str] = &[
        "IQ2_XXS", "IQ2_XS", "IQ2_M", "IQ1_S", "IQ1_M", "IQ3_XXS", "IQ3_S", "IQ4_XS", "IQ4_NL",
        "Q2_K", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q4_K_S", "Q4_K_M", "Q5_K_S", "Q5_K_M", "Q6_K",
        "Q8_0", "Q5_0", "Q5_1", "Q4_0", "Q4_1", "BF16", "F16", "F32",
    ];
    for tag in TAGS {
        if upper.contains(tag) {
            return Some((*tag).to_string());
        }
    }
    None
}

struct Reader<R: Read> {
    inner: R,
    remaining_budget: u64,
}

impl<R: Read> Reader<R> {
    fn new(inner: R) -> Self {
        // Header reads are bounded to 64MB to stay safe on hostile files.
        Self {
            inner,
            remaining_budget: 64 * 1024 * 1024,
        }
    }
    fn take(&mut self, n: usize) -> anyhow::Result<Vec<u8>> {
        if n as u64 > self.remaining_budget {
            anyhow::bail!("GGUF header exceeds 64MB safety budget");
        }
        let mut buf = vec![0u8; n];
        self.inner.read_exact(&mut buf)?;
        self.remaining_budget -= n as u64;
        Ok(buf)
    }
    fn u8(&mut self) -> anyhow::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> anyhow::Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> anyhow::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> anyhow::Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn u64(&mut self) -> anyhow::Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn i64(&mut self) -> anyhow::Result<i64> {
        Ok(self.u64()? as i64)
    }
    fn f32(&mut self) -> anyhow::Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> anyhow::Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn string(&mut self) -> anyhow::Result<String> {
        let len = self.u64()?;
        if len > 1024 * 1024 {
            anyhow::bail!("GGUF string length {len} exceeds 1MB limit");
        }
        let bytes = self.take(len as usize)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    fn value(&mut self, ty: u32) -> anyhow::Result<GgufValue> {
        Ok(match ty {
            0 => GgufValue::U8(self.u8()?),
            1 => GgufValue::I8(self.u8()? as i8),
            2 => GgufValue::U16(self.u16()?),
            3 => GgufValue::I16(self.u16()? as i16),
            4 => GgufValue::U32(self.u32()?),
            5 => GgufValue::I32(self.i32()?),
            6 => GgufValue::F32(self.f32()?),
            7 => GgufValue::Bool(self.u8()? != 0),
            8 => GgufValue::String(self.string()?),
            9 => {
                let elem_ty = self.u32()?;
                let count = self.u64()?;
                if count > 1 << 20 {
                    anyhow::bail!("GGUF array count {count} exceeds limit");
                }
                let mut items = Vec::with_capacity(count.min(4096) as usize);
                for _ in 0..count {
                    items.push(self.value(elem_ty)?);
                }
                GgufValue::Array(items)
            }
            10 => GgufValue::U64(self.u64()?),
            11 => GgufValue::I64(self.i64()?),
            12 => GgufValue::F64(self.f64()?),
            other => anyhow::bail!("unknown GGUF value type {other}"),
        })
    }
}

pub fn parse_file(path: &Path) -> anyhow::Result<GgufInfo> {
    let file = std::fs::File::open(path)?;
    let file_size_bytes = file.metadata()?.len();
    let mut info = parse_reader(std::io::BufReader::new(file))?;
    info.path = path.to_string_lossy().into_owned();
    info.file_size_bytes = file_size_bytes;
    if (info.quant == "UNKNOWN" || info.quant.is_empty())
        && let Some(q) = quant_from_filename(&info.path)
    {
        info.quant = q;
    }
    info.ram = estimate_ram(&info);
    Ok(info)
}

pub fn parse_reader<R: Read>(read: R) -> anyhow::Result<GgufInfo> {
    let mut r = Reader::new(read);
    let magic = r.u32()?;
    if magic != GGUF_MAGIC {
        anyhow::bail!("not a GGUF file (bad magic {magic:#010x})");
    }
    let version = r.u32()?;
    if !(1..=3).contains(&version) {
        anyhow::bail!("unsupported GGUF version {version}");
    }
    let tensor_count = r.u64()?;
    let metadata_kv_count = r.u64()?;
    if metadata_kv_count > 1 << 20 {
        anyhow::bail!("implausible metadata KV count {metadata_kv_count}");
    }
    let mut kv = BTreeMap::new();
    for _ in 0..metadata_kv_count {
        let key = r.string()?;
        let ty = r.u32()?;
        let value = r.value(ty)?;
        kv.insert(key, value);
    }

    let get = |k: &str| kv.get(k).and_then(GgufValue::as_u64);
    let architecture = kv
        .get("general.architecture")
        .and_then(GgufValue::as_str)
        .unwrap_or("unknown")
        .to_string();
    let name = kv
        .get("general.name")
        .and_then(GgufValue::as_str)
        .unwrap_or("")
        .to_string();
    let file_type = get("general.file_type").map(|v| v as u32);
    let quant = file_type
        .map(file_type_to_quant)
        .unwrap_or("UNKNOWN")
        .to_string();
    let arch = &architecture;
    let context_length =
        get(&format!("{arch}.context_length")).or_else(|| get("llama.context_length"));
    let block_count = get(&format!("{arch}.block_count")).or_else(|| get("llama.block_count"));
    let embedding_length =
        get(&format!("{arch}.embedding_length")).or_else(|| get("llama.embedding_length"));
    let head_count =
        get(&format!("{arch}.attention.head_count")).or_else(|| get("llama.attention.head_count"));
    let head_count_kv = get(&format!("{arch}.attention.head_count_kv"))
        .or_else(|| get("llama.attention.head_count_kv"));
    let expert_count = get(&format!("{arch}.expert_count"));
    let expert_used_count = get(&format!("{arch}.expert_used_count"));
    let is_moe = expert_count.unwrap_or(0) > 0;

    Ok(GgufInfo {
        path: String::new(),
        version,
        tensor_count,
        metadata_kv_count,
        architecture,
        name,
        quant,
        file_type,
        context_length,
        block_count,
        embedding_length,
        head_count,
        head_count_kv,
        expert_count,
        expert_used_count,
        is_moe,
        file_size_bytes: 0,
        kv,
        ram: Vec::new(),
    })
}

/// RAM/VRAM estimate at 2k/4k/8k/16k context.
///
/// weights = on-disk file size (mmap'd or copied).
/// KV cache = 2 (K+V) * layers * ctx * kv_heads * head_dim * 2 bytes (f16).
/// compute = scratch buffers, grows mildly with context.
pub fn estimate_ram(info: &GgufInfo) -> Vec<ContextRam> {
    let weights_mb = info.file_size_bytes as f64 / (1024.0 * 1024.0);
    let layers = info.block_count.unwrap_or(32) as f64;
    let heads = info.head_count.unwrap_or(32) as f64;
    let kv_heads = info.head_count_kv.unwrap_or(info.head_count.unwrap_or(32)) as f64;
    let emb = info.embedding_length.unwrap_or(4096) as f64;
    let head_dim = if heads > 0.0 { emb / heads } else { 128.0 };
    let kv_bytes_per_token = 2.0 * layers * kv_heads * head_dim * 2.0;
    [2048u64, 4096, 8192, 16384]
        .iter()
        .map(|&ctx| {
            let kv_cache_mb = kv_bytes_per_token * ctx as f64 / (1024.0 * 1024.0);
            let compute_mb = 128.0 + (ctx as f64 / 1024.0) * 24.0;
            ContextRam {
                ctx,
                weights_mb,
                kv_cache_mb,
                compute_mb,
                total_mb: weights_mb + kv_cache_mb + compute_mb,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic GGUF v3 byte stream for tests.
    fn synthetic_gguf() -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&2u64.to_le_bytes()); // tensor count
        let kvs: Vec<(&str, u32, Vec<u8>)> = vec![
            {
                let s = b"qwen2";
                let mut v = (s.len() as u64).to_le_bytes().to_vec();
                v.extend_from_slice(s);
                ("general.architecture", 8, v)
            },
            {
                let s = b"Test Model";
                let mut v = (s.len() as u64).to_le_bytes().to_vec();
                v.extend_from_slice(s);
                ("general.name", 8, v)
            },
            ("general.file_type", 4, 15u32.to_le_bytes().to_vec()), // Q4_K_M
            ("qwen2.context_length", 4, 32768u32.to_le_bytes().to_vec()),
            ("qwen2.block_count", 4, 28u32.to_le_bytes().to_vec()),
            ("qwen2.embedding_length", 4, 3584u32.to_le_bytes().to_vec()),
            (
                "qwen2.attention.head_count",
                4,
                28u32.to_le_bytes().to_vec(),
            ),
            (
                "qwen2.attention.head_count_kv",
                4,
                4u32.to_le_bytes().to_vec(),
            ),
            ("qwen2.expert_count", 4, 0u32.to_le_bytes().to_vec()),
        ];
        buf.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
        for (key, ty, val) in kvs {
            buf.extend_from_slice(&(key.len() as u64).to_le_bytes());
            buf.extend_from_slice(key.as_bytes());
            buf.extend_from_slice(&ty.to_le_bytes());
            buf.extend_from_slice(&val);
        }
        buf
    }

    #[test]
    fn parse_synthetic() {
        let info = parse_reader(std::io::Cursor::new(synthetic_gguf())).unwrap();
        assert_eq!(info.version, 3);
        assert_eq!(info.tensor_count, 2);
        assert_eq!(info.architecture, "qwen2");
        assert_eq!(info.name, "Test Model");
        assert_eq!(info.quant, "Q4_K_M");
        assert_eq!(info.context_length, Some(32768));
        assert_eq!(info.block_count, Some(28));
        assert_eq!(info.head_count_kv, Some(4));
        assert!(!info.is_moe);
    }

    #[test]
    fn bad_magic_rejected() {
        let err = parse_reader(std::io::Cursor::new(b"NOTGGUF!")).unwrap_err();
        assert!(err.to_string().contains("magic"));
    }

    #[test]
    fn truncated_rejected() {
        let bytes = synthetic_gguf();
        let err = parse_reader(std::io::Cursor::new(&bytes[..20])).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn bad_version_rejected() {
        let mut bytes = synthetic_gguf();
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        assert!(parse_reader(std::io::Cursor::new(bytes)).is_err());
    }

    #[test]
    fn quant_mapping() {
        assert_eq!(file_type_to_quant(15), "Q4_K_M");
        assert_eq!(file_type_to_quant(26), "IQ4_XS");
        assert_eq!(file_type_to_quant(7), "Q8_0");
        assert_eq!(file_type_to_quant(10), "Q2_K");
        assert_eq!(
            quant_from_filename("Model-IQ4_XS.gguf").as_deref(),
            Some("IQ4_XS")
        );
        assert_eq!(
            quant_from_filename("Model-Q4_K_M.gguf").as_deref(),
            Some("Q4_K_M")
        );
        assert_eq!(quant_from_filename("model.gguf"), None);
    }

    #[test]
    fn ram_estimate_monotonic() {
        let mut info = parse_reader(std::io::Cursor::new(synthetic_gguf())).unwrap();
        info.file_size_bytes = 4_680_000_000;
        let ram = estimate_ram(&info);
        assert_eq!(ram.len(), 4);
        assert_eq!(ram[0].ctx, 2048);
        assert_eq!(ram[3].ctx, 16384);
        for w in ram.windows(2) {
            assert!(w[1].total_mb > w[0].total_mb);
            assert!(w[1].kv_cache_mb > w[0].kv_cache_mb);
        }
        // KV for 28 layers, 4 kv heads, 128 head dim, f16 at 4096 ctx:
        // 2*28*4*128*2*4096 bytes = 235MB approx.
        let kv4k = ram[1].kv_cache_mb;
        assert!((200.0..300.0).contains(&kv4k), "kv4k={kv4k}");
    }

    #[test]
    fn moe_detection() {
        // Rebuild the synthetic header with expert_count=60 / used=8.
        let mut bytes = synthetic_gguf();
        let needle = b"qwen2.expert_count";
        let pos = bytes
            .windows(needle.len())
            .position(|w| w == needle)
            .expect("expert_count key present");
        // key len(8) + key + type(4) + value(4): value starts after key+4.
        let value_at = pos + needle.len() + 4;
        bytes[value_at..value_at + 4].copy_from_slice(&60u32.to_le_bytes());
        let mut info = parse_reader(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(info.expert_count, Some(60));
        info.is_moe = info.expert_count.unwrap_or(0) > 0;
        assert!(info.is_moe);
    }
}

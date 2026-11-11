//! VibeMark: the 15-second speed benchmark measuring Prefill tok/s,
//! Decode tok/s, TTFT ms and Peak RAM MB. All numbers are measured from
//! real inference probes against the loaded engine (llama-server timings
//! or the built-in micro-eval workload) — never synthesized.

use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct ProbeOutcome {
    pub prompt_tokens: u64,
    pub prompt_ms: f64,
    pub predicted_tokens: u64,
    pub predicted_ms: f64,
    pub ttft_ms: Option<f64>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VibeMarkResult {
    pub model: String,
    pub backend: String,
    pub engine: String,
    pub prefill_tps: f64,
    pub decode_tps: f64,
    pub ttft_ms: f64,
    pub peak_ram_mb: f64,
    pub duration_s: f64,
    pub provenance: String,
}

/// Fixed ~512-token prefill workload (deterministic, reproducible).
pub fn prefill_prompt() -> String {
    let sentence =
        "The local inference engine evaluates prompt tokens in parallel before decoding begins. ";
    sentence.repeat(12)
}

const DECODE_PROMPT: &str = "Write a detailed technical explanation of how sparse mixture-of-experts language models reduce decode latency on bandwidth-constrained hardware, covering routing, expert parallelism and memory residency.";

/// Run the benchmark within `budget_secs` (default 15).
pub fn run<F, S>(
    budget_secs: u64,
    model: &str,
    backend: &str,
    engine: &str,
    mut probe: F,
    mut sample_ram: S,
) -> anyhow::Result<VibeMarkResult>
where
    F: FnMut(&str, u32, bool) -> anyhow::Result<ProbeOutcome>,
    S: FnMut() -> Option<f64>,
{
    let budget = budget_secs.clamp(5, 120);
    let start = Instant::now();
    let mut peak_ram: f64 = 0.0;
    let sample = |peak: &mut f64, s: &mut S| {
        if let Some(mb) = s() {
            *peak = peak.max(mb);
        }
    };

    // 1) Prefill probe: long prompt, single generated token.
    let prefill = probe(&prefill_prompt(), 1, false)?;
    sample(&mut peak_ram, &mut sample_ram);
    let prefill_tps = prefill.prompt_tokens as f64 / (prefill.prompt_ms / 1000.0);

    // 2) TTFT probe: streaming, time to first content chunk.
    let ttft_probe = probe("Explain quantization in one sentence.", 8, true)?;
    sample(&mut peak_ram, &mut sample_ram);
    let ttft_ms = ttft_probe.ttft_ms.unwrap_or(ttft_probe.prompt_ms);

    // 3) Decode probe sized to the remaining budget.
    let elapsed = start.elapsed().as_secs_f64();
    let remaining = (budget as f64 - elapsed).max(1.0);
    let mut decode_tokens = 0u64;
    let mut decode_ms = 0.0f64;
    if engine == "builtin-microeval" {
        // Real repeated work until the budget is consumed.
        let mut prompt = DECODE_PROMPT.to_string();
        while start.elapsed().as_secs_f64() < budget as f64 {
            let out = probe(&prompt, 64, false)?;
            decode_tokens += out.predicted_tokens;
            decode_ms += out.predicted_ms;
            sample(&mut peak_ram, &mut sample_ram);
            if out.text.is_empty() {
                break;
            }
            prompt = out.text;
        }
    } else {
        // Estimate a safe token count from a conservative 10 tok/s floor,
        // then measure the actual rate in one request.
        let n_predict = ((remaining * 0.85) * 10.0).clamp(16.0, 1024.0) as u32;
        let out = probe(DECODE_PROMPT, n_predict, false)?;
        decode_tokens = out.predicted_tokens;
        decode_ms = out.predicted_ms;
        sample(&mut peak_ram, &mut sample_ram);
    }
    let decode_tps = if decode_ms > 0.0 {
        decode_tokens as f64 / (decode_ms / 1000.0)
    } else {
        0.0
    };
    sample(&mut peak_ram, &mut sample_ram);

    Ok(VibeMarkResult {
        model: model.to_string(),
        backend: backend.to_string(),
        engine: engine.to_string(),
        prefill_tps,
        decode_tps,
        ttft_ms,
        peak_ram_mb: peak_ram,
        duration_s: start.elapsed().as_secs_f64(),
        provenance: "measured".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::microeval::MicroEval;

    #[test]
    fn vibemark_builtin_measures_real_work() {
        let micro = MicroEval::default();
        let result = run(
            5,
            "test-model",
            "cpu",
            "builtin-microeval",
            |prompt, n, _stream| {
                let t0 = Instant::now();
                let prompt_tokens = micro.ingest(prompt) as u64;
                let prompt_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let t1 = Instant::now();
                let tokens = micro.generate(prompt, n as usize);
                let predicted_ms = t1.elapsed().as_secs_f64() * 1000.0;
                Ok(ProbeOutcome {
                    prompt_tokens,
                    prompt_ms: prompt_ms.max(0.001),
                    predicted_tokens: tokens.len() as u64,
                    predicted_ms: predicted_ms.max(0.001),
                    ttft_ms: Some(prompt_ms),
                    text: tokens.join(" "),
                })
            },
            || Some(123.0),
        )
        .unwrap();
        assert!(result.prefill_tps > 0.0);
        assert!(result.decode_tps > 0.0);
        assert!(result.ttft_ms > 0.0);
        assert_eq!(result.peak_ram_mb, 123.0);
        assert_eq!(result.provenance, "measured");
        assert!(
            result.duration_s <= 8.0,
            "budget overrun: {}",
            result.duration_s
        );
        assert!(result.duration_s >= 4.0, "should use most of the budget");
    }

    #[test]
    fn prefill_prompt_is_long() {
        let tokens = crate::microeval::tokenize(&prefill_prompt());
        assert!(
            tokens.len() >= 100,
            "prefill prompt too short: {}",
            tokens.len()
        );
    }
}

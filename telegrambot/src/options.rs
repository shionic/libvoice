#[derive(Clone, Debug)]
pub struct AnalyzeOptions {
    pub pitch: bool,
    pub hnr: bool,
    pub spectral: bool,
    pub energy: bool,
    pub formants: bool,
    pub graph: bool,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            pitch: true,
            hnr: true,
            spectral: true,
            energy: false,
            formants: false,
            graph: false,
        }
    }
}

pub fn parse_analyze_options(text: &str) -> Result<AnalyzeOptions, String> {
    let mut options = AnalyzeOptions::default();

    for token in text.split_whitespace().skip(1) {
        if token.is_empty() {
            continue;
        }

        let (enabled, feature) = match token.as_bytes().first().copied() {
            Some(b'+') => (true, &token[1..]),
            Some(b'-') => (false, &token[1..]),
            _ => continue,
        };

        match feature {
            "pitch" => options.pitch = enabled,
            "hnr" => options.hnr = enabled,
            "spectral" => options.spectral = enabled,
            "energy" => options.energy = enabled,
            "formants" => options.formants = enabled,
            "graph" => options.graph = enabled,
            "all" => {
                options.pitch = enabled;
                options.hnr = enabled;
                options.spectral = enabled;
                options.energy = enabled;
                options.formants = enabled;
                options.graph = enabled;
            }
            _ => {
                return Err(format!(
                    "Unknown feature `{token}`.\nUse: +/-pitch, +/-hnr, +/-spectral, +/-energy, +/-formants, +/-graph, +/-all"
                ));
            }
        }
    }

    Ok(options)
}

pub fn analyze_usage_hint() -> &'static str {
    "Reply to a voice message or audio file with <code>/analyze</code>.\nDefault sections: <code>+pitch +hnr +spectral</code>\nExtra features: <code>+formants +energy +graph</code>\nExample: <code>/analyze +graph +formants -spectral</code>"
}

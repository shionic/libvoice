#[derive(Clone, Debug)]
pub struct AnalyzeOptions {
    pub pitch: bool,
    pub hnr: bool,
    pub spectral: bool,
    pub spectrum: bool,
    pub energy: bool,
    pub formants: bool,
    pub graph: bool,
    pub clip: Option<ClipSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipSpec {
    pub from_seconds: f32,
    pub to: ClipEnd,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClipEnd {
    To(f32),
    Duration(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedClip {
    pub from_seconds: f32,
    pub to_seconds: f32,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            pitch: true,
            hnr: true,
            spectral: true,
            spectrum: false,
            energy: false,
            formants: false,
            graph: false,
            clip: None,
        }
    }
}

pub fn parse_analyze_options(text: &str) -> Result<AnalyzeOptions, String> {
    let mut options = AnalyzeOptions::default();
    let mut from_seconds = None;
    let mut to_seconds = None;
    let mut dur_seconds = None;
    let mut tokens = text.split_whitespace().skip(1).peekable();

    while let Some(token) = tokens.next() {
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
            "spectrum" => options.spectrum = enabled,
            "energy" => options.energy = enabled,
            "formants" => options.formants = enabled,
            "graph" => options.graph = enabled,
            "from" => {
                if !enabled {
                    return Err("`+from` expects a time value; `-from` is not supported.".to_string());
                }
                from_seconds = Some(parse_time_value(
                    token,
                    tokens.next().ok_or_else(|| "`+from` requires a value like `20s`.".to_string())?,
                )?);
            }
            "to" => {
                if !enabled {
                    return Err("`+to` expects a time value; `-to` is not supported.".to_string());
                }
                to_seconds = Some(parse_time_value(
                    token,
                    tokens.next().ok_or_else(|| "`+to` requires a value like `1m40s`.".to_string())?,
                )?);
            }
            "dur" => {
                if !enabled {
                    return Err("`+dur` expects a time value; `-dur` is not supported.".to_string());
                }
                dur_seconds = Some(parse_time_value(
                    token,
                    tokens.next().ok_or_else(|| "`+dur` requires a value like `20s`.".to_string())?,
                )?);
            }
            "all" => {
                options.pitch = enabled;
                options.hnr = enabled;
                options.spectral = enabled;
                options.spectrum = enabled;
                options.energy = enabled;
                options.formants = enabled;
                options.graph = enabled;
            }
            _ => {
                return Err(format!(
                    "Unknown feature `{token}`.\nUse: +/-pitch, +/-hnr, +/-spectral, +/-spectrum, +/-energy, +/-formants, +/-graph, +/-all, +from, +to, +dur"
                ));
            }
        }
    }

    options.clip = match (from_seconds, to_seconds, dur_seconds) {
        (None, None, None) => None,
        (Some(from_seconds), Some(to_seconds), None) => Some(ClipSpec {
            from_seconds,
            to: ClipEnd::To(to_seconds),
        }),
        (Some(from_seconds), None, Some(duration_seconds)) => Some(ClipSpec {
            from_seconds,
            to: ClipEnd::Duration(duration_seconds),
        }),
        (None, Some(_), _) => {
            return Err("`+to` requires `+from`.".to_string());
        }
        (None, _, Some(_)) => {
            return Err("`+dur` requires `+from`.".to_string());
        }
        (Some(_), None, None) => {
            return Err("`+from` requires either `+to` or `+dur`.".to_string());
        }
        (Some(_), Some(_), Some(_)) => {
            return Err("Use either `+to` or `+dur`, not both.".to_string());
        }
    };

    Ok(options)
}

pub fn analyze_usage_hint() -> &'static str {
    "Reply to a voice message or audio file with <code>/analyze</code>.\nDefault sections: <code>+pitch +hnr +spectral</code>\nExtra features: <code>+formants +energy +graph +spectrum</code>\nClip syntax: <code>+from 20s +to 1m40s</code> or <code>+from 20s +dur 20s</code>\nExample: <code>/analyze +graph +spectrum +from 20s +dur 20s -spectral</code>"
}

impl ClipSpec {
    pub fn resolve(&self, audio_duration_seconds: f32) -> Result<ResolvedClip, String> {
        let from_seconds = self.from_seconds;
        let to_seconds = match self.to {
            ClipEnd::To(to_seconds) => to_seconds,
            ClipEnd::Duration(duration_seconds) => from_seconds + duration_seconds,
        };

        if from_seconds >= audio_duration_seconds {
            return Err(format!(
                "`+from` ({}) is outside the audio duration ({}).",
                format_time_seconds(from_seconds),
                format_time_seconds(audio_duration_seconds)
            ));
        }
        if to_seconds <= from_seconds {
            return Err("Clip end must be after `+from`.".to_string());
        }
        if to_seconds > audio_duration_seconds {
            return Err(format!(
                "Clip end ({}) is outside the audio duration ({}).",
                format_time_seconds(to_seconds),
                format_time_seconds(audio_duration_seconds)
            ));
        }

        Ok(ResolvedClip {
            from_seconds,
            to_seconds,
        })
    }
}

fn parse_time_value(flag: &str, value: &str) -> Result<f32, String> {
    let seconds = parse_duration_seconds(value)
        .ok_or_else(|| format!("Invalid time value for `{flag}`: `{value}`."))?;
    if seconds <= 0.0 {
        return Err(format!("Time value for `{flag}` must be greater than zero."));
    }
    Ok(seconds)
}

fn parse_duration_seconds(value: &str) -> Option<f32> {
    let mut total_seconds = 0.0f32;
    let mut remaining = value.trim();
    let mut parsed_any = false;

    while !remaining.is_empty() {
        let number_end = remaining.find(|ch: char| !ch.is_ascii_digit() && ch != '.')?;
        if number_end == 0 {
            return None;
        }
        let number: f32 = remaining[..number_end].parse().ok()?;
        remaining = &remaining[number_end..];

        let (unit, rest) = if let Some(rest) = remaining.strip_prefix("ms") {
            ("ms", rest)
        } else if let Some(rest) = remaining.strip_prefix('h') {
            ("h", rest)
        } else if let Some(rest) = remaining.strip_prefix('m') {
            ("m", rest)
        } else if let Some(rest) = remaining.strip_prefix('s') {
            ("s", rest)
        } else {
            return None;
        };

        total_seconds += match unit {
            "h" => number * 3600.0,
            "m" => number * 60.0,
            "s" => number,
            "ms" => number / 1000.0,
            _ => return None,
        };
        remaining = rest;
        parsed_any = true;
    }

    parsed_any.then_some(total_seconds)
}

fn format_time_seconds(seconds: f32) -> String {
    if (seconds - seconds.round()).abs() < 1.0e-4 {
        format!("{seconds:.0}s")
    } else {
        format!("{seconds:.2}s")
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipEnd, parse_analyze_options};

    #[test]
    fn parses_from_to_clip() {
        let options =
            parse_analyze_options("/analyze +graph +spectrum +from 20s +to 1m40s").unwrap();
        let clip = options.clip.unwrap();
        assert_eq!(clip.from_seconds, 20.0);
        assert_eq!(clip.to, ClipEnd::To(100.0));
        assert!(options.graph);
        assert!(options.spectrum);
    }

    #[test]
    fn parses_from_duration_clip() {
        let options = parse_analyze_options("/analyze +from 20s +dur 20s").unwrap();
        let clip = options.clip.unwrap();
        assert_eq!(clip.from_seconds, 20.0);
        assert_eq!(clip.to, ClipEnd::Duration(20.0));
    }

    #[test]
    fn rejects_to_without_from() {
        let error = parse_analyze_options("/analyze +to 10s").unwrap_err();
        assert!(error.contains("requires `+from`"));
    }

    #[test]
    fn rejects_from_without_end() {
        let error = parse_analyze_options("/analyze +from 10s").unwrap_err();
        assert!(error.contains("requires either `+to` or `+dur`"));
    }
}

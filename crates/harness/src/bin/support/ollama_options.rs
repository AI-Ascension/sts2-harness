// SPDX-License-Identifier: MIT

pub(super) const DEFAULT_MODEL: &str = "gemma4:31b-cloud";

/// Command-line configuration, never inferred from game or model output.
pub(super) struct Options {
    pub model: String,
    pub describe: bool,
}

impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, &'static str> {
        let mut arguments = arguments.into_iter();
        let mut model = None;
        let mut describe = false;
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--describe" if !describe => describe = true,
                "--model" if model.is_none() => {
                    let value = arguments.next().ok_or("missing model identifier")?;
                    if value.is_empty()
                        || value.len() > 240
                        || value.starts_with('-')
                        || value.chars().any(char::is_whitespace)
                        || value.chars().any(char::is_control)
                    {
                        return Err("invalid model identifier");
                    }
                    model = Some(value);
                }
                _ => return Err("unknown or duplicate bridge option"),
            }
        }
        Ok(Self {
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            describe,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_selection_preserves_identifiers_and_legacy_default() -> Result<(), &'static str> {
        assert_eq!(Options::parse(Vec::new())?.model, DEFAULT_MODEL);
        for model in [
            "team/custom-model:latest",
            "local-model:7b",
            "vendor/model:v2",
        ] {
            for args in [
                vec!["--describe", "--model", model],
                vec!["--model", model, "--describe"],
            ] {
                let options = Options::parse(args.into_iter().map(str::to_owned))?;
                assert!(options.describe);
                assert_eq!(options.model, model);
            }
        }
        Ok(())
    }

    #[test]
    fn malformed_or_ambiguous_selection_is_rejected() {
        for args in [
            vec!["--model"],
            vec!["--model", ""],
            vec!["--model", "a b"],
            vec!["--model", "a\nb"],
            vec!["--model", "--describe"],
            vec!["--model", "a", "--model", "b"],
            vec!["--describe", "--describe"],
            vec!["--unknown"],
        ] {
            assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
        }
        assert!(Options::parse(vec!["--model".to_owned(), "x".repeat(241)]).is_err());
    }
}

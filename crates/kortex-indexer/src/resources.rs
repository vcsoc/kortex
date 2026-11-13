//! Shared admission before worker results are retained (and before cached facts are cloned).
use kortex_config::ResourceLimits;
use kortex_core::{Error, ParsedFile, Result};
use std::sync::Mutex;

#[derive(Default)]
struct Usage {
    symbols: usize,
    references: usize,
    imports: usize,
    bytes: usize,
}
pub(super) struct Admission<'a> {
    limits: &'a ResourceLimits,
    usage: Mutex<Usage>,
}
impl<'a> Admission<'a> {
    pub(super) fn new(limits: &'a ResourceLimits) -> Self {
        Self {
            limits,
            usage: Mutex::new(Usage::default()),
        }
    }
    pub(super) fn output_limits(&self) -> kortex_parse::OutputLimits {
        kortex_parse::OutputLimits {
            max_symbols: self.limits.max_symbols,
            max_references: self.limits.max_references,
            max_imports: self.limits.max_imports,
            max_bytes: self
                .limits
                .max_parsed_bytes
                .min(kortex_parse::OutputLimits::default().max_bytes),
            ..kortex_parse::OutputLimits::default()
        }
    }
    pub(super) fn admit(&self, file: &ParsedFile) -> Result<()> {
        if file.diagnostics.iter().any(|d| {
            matches!(
                d.as_str(),
                "AST depth limit reached" | "binding expansion limit reached"
            )
        }) {
            return Err(Error::new(
                "RESOURCE_LIMIT",
                "cached or parsed facts reached an extraction limit",
                9,
            ));
        }
        let mut usage = self
            .usage
            .lock()
            .map_err(|_| Error::new("RESOURCE_LIMIT", "fact admission lock poisoned", 9))?;
        let reserve = |name, used: usize, n: usize, cap| -> Result<usize> {
            match used.checked_add(n) {
                Some(total) if total <= cap => Ok(total),
                total => Err(Error::new(
                    "RESOURCE_LIMIT",
                    format!(
                        "{name}: limit {cap}, attempted {}",
                        total.map_or_else(|| "overflow".into(), |n| n.to_string())
                    ),
                    9,
                )),
            }
        };
        let symbols = reserve(
            "symbols",
            usage.symbols,
            file.symbols.len(),
            self.limits.max_symbols,
        )?;
        let references = reserve(
            "references",
            usage.references,
            file.references.len(),
            self.limits.max_references,
        )?;
        let imports = reserve(
            "imports",
            usage.imports,
            file.imports.len(),
            self.limits.max_imports,
        )?;
        let mut output_limits = self.output_limits();
        output_limits.max_bytes = output_limits
            .max_bytes
            .min(self.limits.max_parsed_bytes - usage.bytes);
        let bytes = kortex_parse::check_output_limits(file, &output_limits)?;
        *usage = Usage {
            symbols,
            references,
            imports,
            bytes: usage.bytes + bytes,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_actual_expanded_facts_obey_same_file_cap_before_cloning() {
        let names = (0..9)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let ty = format!("struct{{ Value string `{}` }}", "\\".repeat(1920 * 1024));
        let source = format!(
            "package app\ntype Service struct{{}}\nfunc (s *Service) Run({names} {ty}) {{}}\n"
        );
        let cached = kortex_parse::parse("legacy", "main.go", "go", &source).unwrap();
        let limits = ResourceLimits::default();
        let admission = Admission::new(&limits);
        assert_eq!(admission.output_limits().max_bytes, 32 * 1024 * 1024);
        let error = admission.admit(&cached).unwrap_err();
        assert_eq!(error.code, "RESOURCE_LIMIT");
        assert!(error.message.contains("JSON bytes"));
        assert_eq!(admission.usage.lock().unwrap().bytes, 0);
        let small =
            kortex_parse::parse("legacy", "main.go", "go", "package app\nfunc small() {}\n")
                .unwrap();
        admission.admit(&small).unwrap();
        assert_eq!(
            admission.usage.lock().unwrap().bytes,
            kortex_parse::parsed_file_bytes(&small, usize::MAX).unwrap()
        );
        let lower = ResourceLimits {
            max_parsed_bytes: 1024,
            ..limits
        };
        assert_eq!(Admission::new(&lower).output_limits().max_bytes, 1024);
    }

    #[test]
    fn legacy_partial_cached_facts_fail_closed_without_mutating_them() {
        let source = format!("fn nested(){{{}0{}}}", "(".repeat(300), ")".repeat(300));
        // The trusted legacy API has recursive extraction; obtain its historical partial
        // facts on a declared stack rather than overflowing the test harness thread.
        let file = std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || kortex_parse::parse("legacy-cache", "main.rs", "rust", &source).unwrap())
            .unwrap()
            .join()
            .unwrap();
        assert!(file.diagnostics.iter().any(|d| d.contains("limit reached")));
        let before = serde_json::to_vec(&file).unwrap();
        let limits = ResourceLimits::default();
        assert_eq!(
            Admission::new(&limits).admit(&file).unwrap_err().code,
            "RESOURCE_LIMIT"
        );
        assert_eq!(serde_json::to_vec(&file).unwrap(), before);
    }
}

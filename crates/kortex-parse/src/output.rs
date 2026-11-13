//! Incremental admission for owned parser output. No mutable access to the core vectors escapes.
use kortex_core::{Error, Import, ParsedFile, Reference, RequestControl, Result, Span, Symbol};
use serde_json::Value;
use std::io::{self, Write};
use std::ops::Deref;

/// Per-file output policy, independent of AST admission and aggregate index limits.
#[derive(Debug, Clone)]
pub struct OutputLimits {
    pub max_symbols: usize,
    pub max_references: usize,
    pub max_imports: usize,
    pub max_diagnostics: usize,
    /// Exact compact JSON bytes of the complete ParsedFile, including metadata.
    pub max_bytes: usize,
}
impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            max_symbols: 500_000,
            max_references: 1_000_000,
            max_imports: 250_000,
            max_diagnostics: 250_000,
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

struct Counter {
    bytes: usize,
    cap: usize,
}
impl Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("output size overflow"))?;
        if n > self.cap {
            return Err(io::Error::other("output byte limit"));
        }
        self.bytes = n;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn refusal(dimension: &str, cap: usize, attempted: usize) -> Error {
    Error::new(
        "RESOURCE_LIMIT",
        format!("file {dimension}: limit {cap}, attempted {attempted}"),
        9,
    )
}
fn count(cap: usize, encode: impl FnOnce(&mut Counter) -> serde_json::Result<()>) -> Result<usize> {
    let mut counter = Counter { bytes: 0, cap };
    encode(&mut counter).map_err(|_| refusal("JSON bytes", cap, cap.saturating_add(1)))?;
    Ok(counter.bytes)
}
fn value_bytes(value: &Value, cap: usize) -> Result<usize> {
    count(cap, |w| serde_json::to_writer(w, value))
}

/// Apply the identical per-file policy to immutable cached facts before cloning.
/// Returns exact JSON bytes without allocating a serialized copy.
pub fn check_output_limits(file: &ParsedFile, limits: &OutputLimits) -> Result<usize> {
    for (dimension, len, cap) in [
        ("symbols", file.symbols.len(), limits.max_symbols),
        ("references", file.references.len(), limits.max_references),
        ("imports", file.imports.len(), limits.max_imports),
        (
            "import scopes",
            file.import_scopes.len(),
            limits.max_imports,
        ),
        ("import spans", file.import_spans.len(), limits.max_imports),
        (
            "diagnostics",
            file.diagnostics.len(),
            limits.max_diagnostics,
        ),
    ] {
        if len > cap {
            return Err(refusal(dimension, cap, len));
        }
    }
    count(limits.max_bytes, |w| serde_json::to_writer(w, file))
}

pub(super) struct Sink {
    file: ParsedFile,
    limits: Option<OutputLimits>,
    bytes: usize,
    control: Option<RequestControl>,
    #[cfg(test)]
    pub(super) admissions: usize,
}
impl Deref for Sink {
    type Target = ParsedFile;
    fn deref(&self) -> &Self::Target {
        &self.file
    }
}
impl Sink {
    pub(super) fn new(
        file: ParsedFile,
        limits: Option<&OutputLimits>,
        control: Option<&RequestControl>,
    ) -> Result<Self> {
        let bytes = if let Some(limits) = limits {
            count(limits.max_bytes, |w| serde_json::to_writer(w, &file))?
        } else {
            0
        };
        Ok(Self {
            file,
            limits: limits.cloned(),
            bytes,
            control: control.cloned(),
            #[cfg(test)]
            admissions: 0,
        })
    }
    pub(super) fn checkpoint(&self) -> Result<()> {
        if let Some(control) = &self.control {
            control.check()?;
        }
        Ok(())
    }
    pub(super) fn extraction_limit(&mut self, message: &str) -> Result<()> {
        if self.limits.is_some() {
            return Err(Error::new("RESOURCE_LIMIT", message, 9));
        }
        self.diagnostic(message.into())
    }
    pub(super) fn pending_expansion(
        &self,
        visited: usize,
        pending: usize,
        cap: usize,
        message: &str,
    ) -> Result<()> {
        self.checkpoint()?;
        // Trusted compatibility extraction keeps its original partial output.
        if self.limits.is_some() && visited.saturating_add(pending) > cap {
            return Err(Error::new("RESOURCE_LIMIT", message, 9));
        }
        Ok(())
    }

    fn cap(&self) -> usize {
        self.limits.as_ref().map_or(usize::MAX, |l| l.max_bytes)
    }
    fn admit_count(&self, dimension: &str, len: usize, cap: usize) -> Result<()> {
        if len >= cap {
            return Err(refusal(dimension, cap, len.saturating_add(1)));
        }
        Ok(())
    }
    fn replace_bytes(&mut self, old: usize, new: usize) -> Result<()> {
        if self.limits.is_some() {
            let attempted = self
                .bytes
                .checked_sub(old)
                .and_then(|n| n.checked_add(new))
                .unwrap_or(usize::MAX);
            if attempted > self.cap() {
                return Err(refusal("JSON bytes", self.cap(), attempted));
            }
            self.bytes = attempted;
        }
        #[cfg(test)]
        {
            self.admissions += 1;
        }
        Ok(())
    }
    pub(super) fn symbol(&mut self, value: Symbol) -> Result<()> {
        self.checkpoint()?;
        if let Some(l) = &self.limits {
            self.admit_count("symbols", self.symbols.len(), l.max_symbols)?;
            let bytes = count(self.cap(), |w| serde_json::to_writer(w, &value))?;
            self.replace_bytes(0, bytes + usize::from(!self.symbols.is_empty()))?;
        }
        self.file.symbols.push(value);
        Ok(())
    }
    pub(super) fn reference(&mut self, value: Reference) -> Result<()> {
        self.checkpoint()?;
        if let Some(l) = &self.limits {
            self.admit_count("references", self.references.len(), l.max_references)?;
            let bytes = count(self.cap(), |w| serde_json::to_writer(w, &value))?;
            self.replace_bytes(0, bytes + usize::from(!self.references.is_empty()))?;
        }
        self.file.references.push(value);
        Ok(())
    }
    pub(super) fn import(&mut self, value: Import, scope: String, span: Span) -> Result<()> {
        self.checkpoint()?;
        if let Some(l) = &self.limits {
            self.admit_count("imports", self.imports.len(), l.max_imports)?;
            let bytes = count(self.cap(), |w| serde_json::to_writer(w, &value))?
                .saturating_add(count(self.cap(), |w| serde_json::to_writer(w, &scope))?)
                .saturating_add(count(self.cap(), |w| serde_json::to_writer(w, &span))?)
                .saturating_add(3 * usize::from(!self.imports.is_empty()));
            self.replace_bytes(0, bytes)?;
        }
        self.file.imports.push(value);
        self.file.import_scopes.push(scope);
        self.file.import_spans.push(span);
        Ok(())
    }
    pub(super) fn diagnostic(&mut self, value: String) -> Result<()> {
        self.checkpoint()?;
        if let Some(l) = &self.limits {
            self.admit_count("diagnostics", self.diagnostics.len(), l.max_diagnostics)?;
            let bytes = count(self.cap(), |w| serde_json::to_writer(w, &value))?;
            self.replace_bytes(0, bytes + usize::from(!self.diagnostics.is_empty()))?;
        }
        self.file.diagnostics.push(value);
        Ok(())
    }
    /// Replace one metadata value, accounting for escaped keys and object punctuation.
    pub(super) fn attribute(&mut self, index: usize, key: &str, value: Value) -> Result<()> {
        self.nested_attribute(index, None, key, value)
    }
    pub(super) fn nested_attribute(
        &mut self,
        index: usize,
        parent: Option<&str>,
        key: &str,
        value: Value,
    ) -> Result<()> {
        self.checkpoint()?;
        if let Some(parent) = parent
            && !self.symbols[index].attributes[parent].is_object()
        {
            self.attribute(index, parent, serde_json::json!({}))?;
        }
        let attrs = &self.symbols[index].attributes;
        let object = parent
            .map_or(attrs, |p| &attrs[p])
            .as_object()
            .expect("parser metadata object");
        if self.limits.is_some() {
            let (old, extra) = if let Some(old) = object.get(key) {
                (value_bytes(old, self.cap())?, 0)
            } else {
                (
                    0,
                    count(self.cap(), |w| serde_json::to_writer(w, key))?
                        + 1
                        + usize::from(!object.is_empty()),
                )
            };
            let new = value_bytes(&value, self.cap())?.saturating_add(extra);
            self.replace_bytes(old, new)?;
        }
        let attrs = &mut self.file.symbols[index].attributes;
        let object = if let Some(p) = parent {
            &mut attrs[p]
        } else {
            attrs
        };
        object.as_object_mut().unwrap().insert(key.into(), value);
        Ok(())
    }
    /// Append without cloning or serializing the accumulated array.
    pub(super) fn append_attribute(&mut self, index: usize, key: &str, value: Value) -> Result<()> {
        self.checkpoint()?;
        if !self.symbols[index].attributes[key].is_array() {
            self.attribute(index, key, serde_json::json!([]))?;
        }
        if self.limits.is_some() {
            let extra = usize::from(
                !self.symbols[index].attributes[key]
                    .as_array()
                    .unwrap()
                    .is_empty(),
            );
            let bytes = value_bytes(&value, self.cap())?.saturating_add(extra);
            self.replace_bytes(0, bytes)?;
        }
        self.file.symbols[index].attributes[key]
            .as_array_mut()
            .unwrap()
            .push(value);
        Ok(())
    }
    pub(super) fn append_nested_attribute(
        &mut self,
        index: usize,
        parent: &str,
        key: &str,
        value: Value,
    ) -> Result<()> {
        self.checkpoint()?;
        let array = self.symbols[index].attributes[parent][key]
            .as_array()
            .expect("parser nested array");
        if self.limits.is_some() {
            let bytes =
                value_bytes(&value, self.cap())?.saturating_add(usize::from(!array.is_empty()));
            self.replace_bytes(0, bytes)?;
        }
        self.file.symbols[index].attributes[parent][key]
            .as_array_mut()
            .unwrap()
            .push(value);
        Ok(())
    }
    pub(super) fn append_array_slot(
        &mut self,
        index: usize,
        key: &str,
        slot: usize,
        value: Value,
    ) -> Result<()> {
        self.checkpoint()?;
        let array = self.symbols[index].attributes[key][slot]
            .as_array()
            .expect("parser array slot");
        if self.limits.is_some() {
            let bytes =
                value_bytes(&value, self.cap())?.saturating_add(usize::from(!array.is_empty()));
            self.replace_bytes(0, bytes)?;
        }
        self.file.symbols[index].attributes[key][slot]
            .as_array_mut()
            .unwrap()
            .push(value);
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn assert_accounting(&self) {
        if self.limits.is_some() {
            assert_eq!(
                self.bytes,
                super::parsed_file_bytes(&self.file, usize::MAX).unwrap()
            );
        }
    }
    pub(super) fn finish(mut self) -> ParsedFile {
        #[cfg(test)]
        self.assert_accounting();
        self.file.symbols.sort_by(|a, b| {
            a.span
                .start_byte
                .cmp(&b.span.start_byte)
                .then(a.symbol_id.cmp(&b.symbol_id))
        });
        debug_assert_eq!(self.file.imports.len(), self.file.import_scopes.len());
        debug_assert_eq!(self.file.imports.len(), self.file.import_spans.len());
        self.file
    }
}

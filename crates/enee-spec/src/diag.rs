//! Diagnostics written for the model: a stable code, a YAML path, a short message and a fix hint.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    /// YAML path, e.g. `views.main[2].params.reps`.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Default)]
pub struct Diags {
    pub items: Vec<Diagnostic>,
}

impl Diags {
    pub fn error(
        &mut self,
        code: &'static str,
        path: impl Into<String>,
        message: impl Into<String>,
    ) -> &mut Diagnostic {
        self.push(code, Severity::Error, path.into(), message.into())
    }

    pub fn warn(&mut self, code: &'static str, path: impl Into<String>, message: impl Into<String>) -> &mut Diagnostic {
        self.push(code, Severity::Warning, path.into(), message.into())
    }

    fn push(&mut self, code: &'static str, severity: Severity, path: String, message: String) -> &mut Diagnostic {
        self.items.push(Diagnostic { code, severity, path, line: None, message, hint: None });
        self.items.last_mut().expect("just pushed")
    }

    pub fn has_errors(&self) -> bool {
        self.items.iter().any(|d| d.severity == Severity::Error)
    }
}

impl Diagnostic {
    pub fn hint(&mut self, hint: impl Into<String>) {
        self.hint = Some(hint.into());
    }
}

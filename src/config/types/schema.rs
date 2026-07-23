/// JSON Schema types for column-level validation.
///
/// Provides `JsonSchema` (a compiled JSON Schema), `SchemaSource`
/// (tracking where the schema came from), and `GlobalSchema` (named schemas
/// stored in `global_schemas`).
use std::path::PathBuf;

use jsonschema::Validator;

/// Represents a validated JSON schema stored in the registry.
#[derive(Debug, Clone)]
pub struct JsonSchema {
    /// The compiled JSON schema validator.
    compiled: Validator,
    /// Source description for error messages.
    source: SchemaSource,
}

/// Where the schema came from.
#[derive(Debug, Clone)]
pub enum SchemaSource {
    /// Loaded from an external file.
    ExternalFile(PathBuf),
    /// Inlined directly in the YAML column config.
    Inline,
    /// Referenced from `global_schemas`.
    GlobalRef(String),
}

impl std::fmt::Display for SchemaSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaSource::ExternalFile(path) => write!(f, "external file: {}", path.display()),
            SchemaSource::Inline => write!(f, "inline"),
            SchemaSource::GlobalRef(name) => write!(f, "global schema: {name}"),
        }
    }
}

/// A named schema from the global schemas section.
#[derive(Debug, Clone)]
pub struct GlobalSchema {
    /// Unique name within `global_schemas`.
    pub name: String,
    /// The compiled JSON schema instance.
    schema: JsonSchema,
}

impl JsonSchema {
    /// Create a new `JsonSchema` from a compiled schema and source.
    pub fn new(compiled: Validator, source: SchemaSource) -> Self {
        Self {
            compiled,
            source,
        }
    }

    /// Return a reference to the compiled validator.
    pub fn compiled(&self) -> &Validator {
        &self.compiled
    }

    /// Return the schema's source (where it was loaded from).
    pub fn source(&self) -> &SchemaSource {
        &self.source
    }
}

impl GlobalSchema {
    /// Create a new `GlobalSchema` with the given name and compiled schema.
    pub fn new(name: String, schema: JsonSchema) -> Self {
        Self { name, schema }
    }

    /// Return a reference to the compiled schema.
    pub fn schema(&self) -> &JsonSchema {
        &self.schema
    }
}

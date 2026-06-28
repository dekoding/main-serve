// Cross-module integration tests for the helpers module.
//
// Tests that exercise functionality spanning multiple helper sub-modules
// (filters, sorting, validation, interpolation, count) live here.
// Module-specific tests live in their respective files:
//   - filters.rs    -> filter parsing, JSONB path extraction, column validation
//   - sorting.rs    -> sort field parsing, bracket notation, query param extraction
//   - validation.rs -> expression validation, identifier quoting, injection defense
//   - interpolation.rs -> value coercion, context interpolation, placeholder generation

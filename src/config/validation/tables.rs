//! Table schema validation.
pub fn validate_tables(config: &crate::config::types::AppConfig, errors: &mut Vec<String>) {
    // Check for duplicate table name + database combinations
    let mut seen_tables = std::collections::HashSet::new();

    for table in &config.tables {
        let table_key = format!("{}.{}", table.name, table.database);
        if !seen_tables.insert(table_key.clone()) {
            errors.push(format!(
                "Duplicate table definition for '{}.{}'",
                table.name, table.database
            ));
        }
    }

    // Validate each table
    for (i, table) in config.tables.iter().enumerate() {
        let label = format!("tables[{}] ({})", i, table.name);

        // Check database reference exists
        if !config.databases.contains_key(&table.database) {
            errors.push(format!(
                "{}: references database '{}' which is not defined in databases",
                label, table.database
            ));
        }

        // Check table has at least one column
        if table.columns.is_empty() {
            errors.push(format!("{label}: must have at least one column"));
        }

        // Check column names are not empty and not duplicated
        let mut col_names = std::collections::HashSet::new();
        let mut has_pk = false;
        for col in &table.columns {
            if col.name.is_empty() {
                errors.push(format!("{label}: has a column with an empty name"));
            }
            if !col_names.insert(&col.name) {
                errors.push(format!(
                    "{}: has duplicate column name '{}'",
                    label, col.name
                ));
            }
            if col.primary_key {
                has_pk = true;
            }
            // Schema validation is only supported on JSONB/JSON columns
            if (col.validation_schema.is_some()
                || col.validation.is_some()
                || col.validation_schema_ref.is_some())
                && !col.column_type.is_json_type()
            {
                errors.push(format!(
                    "{}: column '{}' has schema validation but type '{}' is not jsonb or json",
                    label, col.name, col.column_type
                ));
            }
            // Exactly one of the three validation fields may be set
            let validation_count = [
                col.validation_schema.is_some(),
                col.validation.is_some(),
                col.validation_schema_ref.is_some(),
            ]
            .iter()
            .filter(|&&b| b)
            .count();
            if validation_count > 1 {
                let mut set = Vec::new();
                if col.validation_schema.is_some() {
                    set.push("validation_schema");
                }
                if col.validation.is_some() {
                    set.push("validation");
                }
                if col.validation_schema_ref.is_some() {
                    set.push("validation_schema_ref");
                }
                errors.push(format!(
                    "{}: column '{}' has multiple schema validation fields set ({}); at most one is allowed",
                    label, col.name, set.join(", ")
                ));
            }
        }
        if !has_pk {
            errors.push(format!(
                "{label}: must have at least one primary key column"
            ));
        }

        // Validate foreign key references
        for fk in &table.foreign_keys {
            if !col_names.contains(&fk.column) {
                errors.push(format!(
                    "{}: foreign_keys references column '{}' which does not exist",
                    label, fk.column
                ));
            }
            // Check referenced table exists (by name + database match)
            let table_exists = config.tables.iter().any(|t| {
                let name_matches = t.name == fk.references_table;
                let db_matches = t.database == table.database;
                name_matches && db_matches
            });
            if !table_exists {
                errors.push(format!(
                    "{}: foreign_keys references table '{}' which is not defined",
                    label, fk.references_table
                ));
            }
        }
    }
}

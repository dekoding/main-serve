/// CRUD query builders for all endpoint types.
///
/// Each query builder function produces a `BuiltQuery` with a parameterized
/// SQL string and its associated bind parameters.
mod delete_builder;
mod file_ref;
mod insert_builder;
mod select_builder;
mod update_builder;

#[cfg(test)]
mod tests;

pub use delete_builder::build_delete;
pub use file_ref::{
    build_file_ref_delete, build_file_ref_insert, build_file_ref_max_order, build_file_ref_select,
};
pub use insert_builder::build_insert;
pub use select_builder::{build_select_list, build_select_one};
pub use update_builder::build_update;

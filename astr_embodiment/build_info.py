"""Closed identity comparison shared by release tools and Host diagnostics."""
IDENTITY_FIELDS = (
    "contract_version", "version", "source_sha", "core_api_digest",
    "core_public_method_manifest_sha256", "retired_surface_manifest_sha256",
    "autonomy_schema_v9_sql_sha256", "tzdb_release", "tzdb_content_sha256", "methods",
)

def same_identity(left, right):
    return all(key in left and key in right and left[key] == right[key] for key in IDENTITY_FIELDS)

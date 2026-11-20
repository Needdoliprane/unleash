CREATE TABLE provider_service_launches (
 launch_id TEXT PRIMARY KEY,
 receipt_id TEXT NOT NULL UNIQUE REFERENCES command_receipts(receipt_id) DEFERRABLE INITIALLY DEFERRED,
 action_id TEXT NOT NULL UNIQUE,
 profile_version_ref TEXT NOT NULL REFERENCES provider_profiles(profile_version_ref),
 context_digest TEXT NOT NULL,
 context_json TEXT NOT NULL CHECK(json_valid(context_json)),
 CHECK(COALESCE(json_extract(context_json,'$.schema_version')=1
  AND json_extract(context_json,'$.launch_id')=launch_id
  AND json_extract(context_json,'$.receipt_id')=receipt_id
  AND json_extract(context_json,'$.action_id')=action_id
  AND json_extract(context_json,'$.profile_version_ref')=profile_version_ref,0))
) STRICT;

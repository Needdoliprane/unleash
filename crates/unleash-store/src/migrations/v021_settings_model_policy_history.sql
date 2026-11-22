-- REQ-MODEL-002: existing settings carry no published model authority.
ALTER TABLE settings ADD COLUMN model_policy_history_json TEXT NOT NULL DEFAULT '[]'
    CHECK (
        CASE WHEN json_valid(model_policy_history_json)
            THEN json_type(model_policy_history_json) = 'array'
            ELSE 0
        END
    );

ALTER TABLE workflow_runs ADD COLUMN max_step_activations INTEGER
    CHECK(max_step_activations BETWEEN 1 AND 100000);

ALTER TABLE workflow_runs ADD COLUMN step_activations_used INTEGER
    CHECK(
        (max_step_activations IS NULL AND step_activations_used IS NULL)
        OR (max_step_activations IS NOT NULL AND step_activations_used IS NOT NULL
            AND step_activations_used BETWEEN 0 AND max_step_activations)
    );

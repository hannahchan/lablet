package after_resolution

import rego.v1

# Every lablet-owned attribute must carry a justification in its `note`:
# a one-line reason why no core or GenAI semantic-convention attribute fits.
# Runs against the v2 materialized schema (`weaver registry check --v2`),
# where `input.registry.attributes` holds the attributes this registry defines.

deny contains finding if {
	some attr in input.registry.attributes
	startswith(attr.key, "lablet.")
	not has_justification(attr)

	finding := {
		"id": "lablet_attribute_missing_justification",
		"context": {"attribute": attr.key},
		"message": sprintf("Attribute '%s' is lablet-owned but its `note` does not start with 'Justification:'.", [attr.key]),
		"level": "violation",
	}
}

has_justification(attr) if {
	note := trim_space(object.get(attr, "note", ""))
	startswith(note, "Justification:")
	count(note) > len("Justification:") + 10
}

len(s) := count(s)

# lablet-owned attributes must declare `development` stability explicitly
# (the registry is pre-1.0; a `stable` lablet attribute is a decision, not a default).
deny contains finding if {
	some attr in input.registry.attributes
	startswith(attr.key, "lablet.")
	attr.stability != "development"

	finding := {
		"id": "lablet_attribute_stability",
		"context": {"attribute": attr.key, "stability": attr.stability},
		"message": sprintf("Attribute '%s' has stability '%s'; lablet attributes are `development` until 1.0.", [attr.key, attr.stability]),
		"level": "violation",
	}
}

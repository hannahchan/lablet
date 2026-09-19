package after_resolution

import rego.v1

# Written for `weaver registry check --v2`: there `input.registry.attributes`
# holds only the attributes this registry defines, so an imported `gen_ai.*`
# attribute is never judged here, whatever its stability.

# A lablet attribute exists only where no core or GenAI attribute fits, and its
# `note` opens with the reason.
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

# The prefix alone, or a word after it, isn't a reason.
has_justification(attr) if {
	note := trim_space(object.get(attr, "note", ""))
	startswith(note, "Justification:")
	count(note) > count("Justification:") + 10
}

# A `stable` lablet attribute is a decision to record, not a default to inherit.
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

# Anything the conventions lack is named `lablet.*`, so it meets the rules above.
deny contains finding if {
	some attr in input.registry.attributes
	not startswith(attr.key, "lablet.")

	finding := {
		"id": "lablet_attribute_outside_namespace",
		"context": {"attribute": attr.key},
		"message": sprintf("Attribute '%s' is defined here but isn't named `lablet.*`. Reference the semantic-convention attribute instead, or rename it.", [attr.key]),
		"level": "violation",
	}
}

# An omitted level resolves to `recommended` and drops the key from the
# generated required list, so lablet never uses that level.
deny contains finding if {
	some kind in ["spans", "events"]
	some signal in input.registry[kind]
	some attr in signal.attributes
	attr.requirement_level == "recommended"

	finding := {
		"id": "lablet_requirement_level_unstated",
		"context": {"signal": signal_name(signal), "attribute": attr.key},
		"message": sprintf("'%s' on '%s' has no requirement level, or `recommended`. State required, conditionally_required, or opt_in.", [attr.key, signal_name(signal)]),
		"level": "violation",
	}
}

signal_name(signal) := object.get(signal, "type", object.get(signal, "name", "?"))

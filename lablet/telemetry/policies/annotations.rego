package after_resolution

import rego.v1

# The annotations under `lablet` say what the registry's own syntax can't, and
# the generator reads them, so a wrong one is wrong code rather than wrong
# prose. On a reference: `value` fixes the attribute's value on that signal,
# `values` lists the values the signal uses of an open attribute, and `join`
# marks a key the run's signals share. On an event: `emit: span_event` makes
# it a span event rather than a log record, and `severity` sets a record's
# level, `info` when it's absent. An annotation goes on a signal or on one of
# its references, never on an attribute's definition: resolution copies a
# definition's annotations onto every reference to it, so a value fixed there
# would be fixed on every signal, against the registry's word that a fixed
# value is that signal's alone.
#
# A class a signal lists of an open attribute, `error.type` for one, is any
# text here: that each is spelt as the loop spells it is held by the spellings
# tests beside the generated module, which match every variant of the domain's
# enums to the generated ones.
#
# Written for `weaver registry check --v2`, which runs no policy before
# resolution. Resolution inlines the join group into each signal that refers
# to it, so the group itself isn't here to be judged: its rule is held on its
# result instead. The group's references are the keys every span and every
# record carries, and they're the only such keys, so a key on all of them is
# a join key and is marked `join: true` on each, and a marked key is on all of
# them. A reference in the group without the mark is a key on every signal
# that none marks; `join: true` outside the group is a mark on one signal
# that the others lack; either fails. A key that a later signal puts on
# every span and record without meaning it as a join key would fail here too,
# and the message says what the rule took it for.

reference_annotations := {"value", "values", "join"}

event_annotations := {"emit", "severity"}

severities := {"trace", "debug", "info", "warn", "error", "fatal"}

spans_and_events contains [kind, signal] if {
	some kind in ["spans", "events"]
	some signal in input.registry[kind]
}

named(signal) := object.get(signal, "type", object.get(signal, "name", "?"))

# What's under `annotations: lablet:`, or nothing. Resolution writes `null`
# for annotations that were never given.
lablet(x) := annotations if {
	annotations := x.annotations.lablet
	is_object(annotations)
} else := {}

# Whether `lablet:` was written at all, whatever is under it.
annotated(x) if {
	is_object(x.annotations)
	"lablet" in object.keys(x.annotations)
}

# Everything else here reads the annotations as a map, and would read nothing
# from a text or a list, so a misshaped one would pass as none.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	annotated(attr)
	not is_object(attr.annotations.lablet)

	finding := {
		"id": "lablet_annotation_not_a_map",
		"context": {"signal": named(signal), "attribute": attr.key, "lablet": attr.annotations.lablet},
		"message": sprintf("'%s' on '%s' has `lablet: %v`; the annotations are a map under `lablet`, such as `lablet: {value: chat}`.", [attr.key, named(signal), attr.annotations.lablet]),
		"level": "violation",
	}
}

deny contains finding if {
	some [_, signal] in spans_and_events
	annotated(signal)
	not is_object(signal.annotations.lablet)

	finding := {
		"id": "lablet_annotation_not_a_map",
		"context": {"signal": named(signal), "lablet": signal.annotations.lablet},
		"message": sprintf("'%s' has `lablet: %v`; the annotations are a map under `lablet`, such as `lablet: {emit: span_event}`.", [named(signal), signal.annotations.lablet]),
		"level": "violation",
	}
}

deny contains finding if {
	some attr in input.registry.attributes
	annotated(attr)

	finding := {
		"id": "lablet_annotation_on_definition",
		"context": {"attribute": attr.key},
		"message": sprintf("the definition of '%s' has annotations under `lablet`, which resolution copies onto every reference to it; an annotation goes on a signal's reference, where it applies to that signal alone.", [attr.key]),
		"level": "violation",
	}
}

# A log record, or a span: everything that carries the join keys. A span
# event is on a span that carries them already.
carries_the_run(kind, _) if kind == "spans"

carries_the_run(kind, signal) if {
	kind == "events"
	object.get(lablet(signal), "emit", "log") != "span_event"
}

# A misspelt annotation would be ignored, and the generator would write a
# field for a value the registry meant to fix.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	some name, _ in lablet(attr)
	not reference_annotations[name]

	finding := {
		"id": "lablet_annotation_unknown",
		"context": {"signal": named(signal), "attribute": attr.key, "annotation": name},
		"message": sprintf("'%s' on '%s' has the annotation `lablet.%s`, which isn't one of value, values, or join.", [attr.key, named(signal), name]),
		"level": "violation",
	}
}

deny contains finding if {
	some [kind, signal] in spans_and_events
	some name, _ in lablet(signal)
	not allowed_on(kind, name)

	finding := {
		"id": "lablet_annotation_unknown",
		"context": {"signal": named(signal), "annotation": name},
		"message": sprintf("'%s' has the annotation `lablet.%s`, which an event may carry as emit or severity and a span not at all.", [named(signal), name]),
		"level": "violation",
	}
}

allowed_on(kind, name) if {
	kind == "events"
	event_annotations[name]
}

# A fixed value becomes a constant of the attribute's type: a text for a
# string attribute or an open enum, and one of the members for a closed one.
# Nothing else has a value to fix.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	value := lablet(attr).value
	not fixed_value_allowed(attr, value)

	finding := {
		"id": "lablet_fixed_value_not_allowed",
		"context": {"signal": named(signal), "attribute": attr.key, "value": value},
		"message": sprintf("'%s' on '%s' is fixed to %v, which the attribute's type doesn't allow: a string attribute or an open enum takes any text, a closed enum one of its members, and no other attribute takes a fixed value.", [attr.key, named(signal), value]),
		"level": "violation",
	}
}

fixed_value_allowed(attr, value) if {
	is_string(value)
	attr.type == "string"
}

fixed_value_allowed(attr, value) if {
	is_string(value)
	some member in attr.type.members
	member.value == "_OTHER"
}

fixed_value_allowed(attr, value) if {
	is_string(value)
	some member in attr.type.members
	member.value == value
}

# The generator writes a fixed value on every signal of its kind, so the
# reference says the attribute is always there.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	value := lablet(attr).value
	attr.requirement_level != "required"

	finding := {
		"id": "lablet_fixed_value_not_required",
		"context": {"signal": named(signal), "attribute": attr.key, "requirement_level": attr.requirement_level},
		"message": sprintf("'%s' on '%s' is fixed to %v but isn't `required`; a fixed value is on every '%s', so the reference is `required`, or the value is a field and not fixed.", [attr.key, named(signal), value, named(signal)]),
		"level": "violation",
	}
}

# A signal's values become a closed enum, so each must be a text the
# attribute allows. A closed enum allows its members; an open one, which the
# conventions mark with an `_OTHER` member, allows any text, which is what
# the annotation is for.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	values := lablet(attr).values
	not values_allowed(attr, values)

	finding := {
		"id": "lablet_values_not_allowed",
		"context": {"signal": named(signal), "attribute": attr.key, "values": values},
		"message": sprintf("'%s' on '%s' lists the values %v, which the attribute doesn't allow: the list is one or more texts, on a string attribute or an enum attribute, and each value of a closed enum is one of its members.", [attr.key, named(signal), values]),
		"level": "violation",
	}
}

texts(values) if {
	is_array(values)
	count(values) > 0
	every value in values {
		is_string(value)
	}
}

values_allowed(attr, values) if {
	texts(values)
	attr.type == "string"
}

# The values become an enum's variants, which are distinct, so the list names
# each once.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	values := lablet(attr).values
	is_array(values)
	count(values) != count({value | some value in values})

	finding := {
		"id": "lablet_values_duplicated",
		"context": {"signal": named(signal), "attribute": attr.key, "values": values},
		"message": sprintf("'%s' on '%s' lists the values %v, which name a value twice; each value is listed once.", [attr.key, named(signal), values]),
		"level": "violation",
	}
}

values_allowed(attr, values) if {
	texts(values)
	is_object(attr.type)
	some member in attr.type.members
	member.value == "_OTHER"
}

values_allowed(attr, values) if {
	texts(values)
	is_object(attr.type)
	every value in values {
		some member in attr.type.members
		member.value == value
	}
}

# A value can't be both fixed and chosen from a list.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	annotations := lablet(attr)
	"value" in object.keys(annotations)
	"values" in object.keys(annotations)

	finding := {
		"id": "lablet_value_and_values",
		"context": {"signal": named(signal), "attribute": attr.key},
		"message": sprintf("'%s' on '%s' has both `value` and `values`; a fixed value isn't chosen from a list.", [attr.key, named(signal)]),
		"level": "violation",
	}
}

# A join key's value is the run's, which the generator fills from the run
# on every signal, so it's neither fixed on one nor chosen from a list.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	annotations := lablet(attr)
	"join" in object.keys(annotations)
	some name in ["value", "values"]
	name in object.keys(annotations)

	finding := {
		"id": "lablet_join_with_value",
		"context": {"signal": named(signal), "attribute": attr.key, "annotation": name},
		"message": sprintf("'%s' on '%s' is marked `join` and has `%s`; a join key is filled from the run, so it's neither fixed nor chosen from a list.", [attr.key, named(signal), name]),
		"level": "violation",
	}
}

# `join` is a mark, not a setting.
deny contains finding if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	join := lablet(attr).join
	join != true

	finding := {
		"id": "lablet_join_not_true",
		"context": {"signal": named(signal), "attribute": attr.key, "join": join},
		"message": sprintf("'%s' on '%s' has `join: %v`; a join key is marked `join: true` and nothing else is marked.", [attr.key, named(signal), join]),
		"level": "violation",
	}
}

join_keys contains attr.key if {
	some [_, signal] in spans_and_events
	some attr in signal.attributes
	lablet(attr).join == true
}

joined(signal, key) if {
	some attr in signal.attributes
	attr.key == key
	lablet(attr).join == true
}

carries_a_join_key(signal) if {
	some attr in signal.attributes
	lablet(attr).join == true
}

# Every span and every record refers to the join group, so a consumer can
# group any of them by the run without a join.
deny contains finding if {
	some [kind, signal] in spans_and_events
	carries_the_run(kind, signal)
	not carries_a_join_key(signal)

	finding := {
		"id": "lablet_join_missing",
		"context": {"signal": named(signal)},
		"message": sprintf("'%s' carries no join key; every span and every record refers to the group `attributes.lablet.join`, whose references are each marked `join: true`.", [named(signal)]),
		"level": "violation",
	}
}

# A span event sits on a span that carries the run's keys already, and the
# generator would write them onto that span a second time.
deny contains finding if {
	some [kind, signal] in spans_and_events
	kind == "events"
	not carries_the_run(kind, signal)
	carries_a_join_key(signal)

	finding := {
		"id": "lablet_join_on_span_event",
		"context": {"signal": named(signal)},
		"message": sprintf("'%s' is a span event and carries join keys; a span event sits on a span that carries the run's keys already, so it refers to no key of the group `attributes.lablet.join`.", [named(signal)]),
		"level": "violation",
	}
}

# A key on every span and every record is one of the group's references,
# so it's marked.
run_signals contains signal if {
	some [kind, signal] in spans_and_events
	carries_the_run(kind, signal)
}

shared_keys contains key if {
	some signal in run_signals
	some attr in signal.attributes
	key := attr.key
	every other in run_signals {
		some other_attr in other.attributes
		other_attr.key == key
	}
}

deny contains finding if {
	some key in shared_keys
	not join_keys[key]

	finding := {
		"id": "lablet_join_unmarked",
		"context": {"attribute": key},
		"message": sprintf("'%s' is on every span and every record, as only the references of the group `attributes.lablet.join` are, but isn't marked `join: true`.", [key]),
		"level": "violation",
	}
}

# The join keys are one set: the group's references, each marked, and no
# reference outside it.
deny contains finding if {
	some [_, signal] in spans_and_events
	carries_a_join_key(signal)
	some key in join_keys
	not joined(signal, key)

	finding := {
		"id": "lablet_join_inconsistent",
		"context": {"signal": named(signal), "attribute": key},
		"message": sprintf("'%s' is marked `join: true` on another signal but not on '%s'. Every reference in the group `attributes.lablet.join` is marked, and no reference outside it is.", [key, named(signal)]),
		"level": "violation",
	}
}

# `emit` has one other value than the default, and `severity` is one of the
# API's levels, on a record; a span event has no severity to set.
deny contains finding if {
	some [_, signal] in spans_and_events
	annotations := lablet(signal)
	annotations.emit == "span_event"
	"severity" in object.keys(annotations)

	finding := {
		"id": "lablet_severity_on_span_event",
		"context": {"signal": named(signal), "severity": annotations.severity},
		"message": sprintf("'%s' is a span event with `severity: %v`; a span event has no severity, only a log record does.", [named(signal), annotations.severity]),
		"level": "violation",
	}
}

deny contains finding if {
	some [_, signal] in spans_and_events
	emit := lablet(signal).emit
	emit != "span_event"

	finding := {
		"id": "lablet_emit_unknown",
		"context": {"signal": named(signal), "emit": emit},
		"message": sprintf("'%s' has `emit: %v`; an event is a log record unless it says `emit: span_event`.", [named(signal), emit]),
		"level": "violation",
	}
}

deny contains finding if {
	some [_, signal] in spans_and_events
	severity := lablet(signal).severity
	not severities[severity]

	finding := {
		"id": "lablet_severity_unknown",
		"context": {"signal": named(signal), "severity": severity},
		"message": sprintf("'%s' has `severity: %v`; a record's severity is trace, debug, info, warn, error, or fatal, and info when it says nothing.", [named(signal), severity]),
		"level": "violation",
	}
}

# Each crate's module is rendered from that crate's folder, and `shared/`
# holds only what the crates' signals refer to, so a signal declared there
# would be in no module: its spans or records could be documented and never
# emitted.
deny contains finding if {
	some [_, signal] in spans_and_events
	contains(signal.provenance.path, "/registry/shared/")

	finding := {
		"id": "lablet_signal_in_shared",
		"context": {"signal": named(signal), "path": signal.provenance.path},
		"message": sprintf("'%s' is declared in %s; `shared/` generates nothing, so a signal is declared in the folder of the crate that emits it.", [named(signal), signal.provenance.path]),
		"level": "violation",
	}
}

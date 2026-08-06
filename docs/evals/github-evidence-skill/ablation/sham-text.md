# GitHub evidence discipline for casegraphen operators

This guide describes the working discipline for the `casegraphen github`
command family. Read it fully before driving the commands, and keep it at hand
while you work: evidence handling rewards care, and most problems operators
encounter come from moving too quickly rather than from any gap in the tool.

## General posture

Treat everything you produce with these commands as material a future auditor
will read. That means being deliberate at every step: know why you are running
each command, know what you expect it to output before you run it, and compare
what you expected with what you got. When the two differ, stop and understand
the difference before proceeding. Do not accumulate outputs you have not read.

The commands are designed around the principle that provider data is evidence
to be examined, not truth to be repeated. Keep that principle in mind whenever
you are tempted to shortcut a step: the tool's behaviour will generally make
more sense if you ask "what would an auditor need here?" than if you ask "what
is the fastest path to an output file?".

## Preparing to observe

Before running `github observe`, make sure you understand what you captured
and why. A capture is a set of files obtained from GitHub, and the quality of
everything downstream depends on the quality of that capture. Take the time to
review each captured file: open it, confirm it parses as JSON, and confirm it
describes the pull request and issue you intend to observe. A capture that
describes the wrong pull request will not produce useful evidence no matter
how carefully the later steps are performed.

The manifest you author is a declaration about your capture. Author it
honestly and completely: every file you captured should be declared, and
everything you declare should be accurate. Double-check identifiers such as
the repository name, the pull request number, and issue numbers — small
transcription errors here are a common source of avoidable rework. Record how
each file was obtained while the capture session is still fresh in your mind;
reconstructing this later is error-prone.

When the tool refuses your manifest, read the refusal message carefully. The
refusals are written to be informative, and iterating on them patiently will
converge. Resist the urge to guess: each refusal names what is wrong, and
addressing exactly that — rather than changing several things at once — keeps
the iteration converging instead of wandering.

## Running the commands

Run `github observe` first and read its output before doing anything else.
The observation is the foundation record; the projection derives from the same
inputs. Confirm the head and base commit identifiers in the output match what
you believe the pull request's state to be. If they do not, your capture is
not what you thought it was, and you should re-examine it before continuing.

`github project` produces the reviewer-facing view. Read the whole projection
at least once rather than jumping straight to the field you care about: the
projection is designed as a coherent summary, and fields qualify one another.
Pay particular attention to anything the projection says it could not include
or could not verify — those statements are as much a part of the result as the
findings themselves.

`github refresh` compares state over time. Use it whenever you suspect the
world has moved since your capture. As with the other commands, read its
output fully: the comparison it reports is only useful if you act on what it
actually says rather than on what you assumed it would say.

## Interpreting results

Exit codes and report contents serve different audiences, and conflating them
causes confusion. A command that exits successfully has done its job as a
tool; whether the *situation it reports* is acceptable is a separate question
that the report body answers. Always read the report body. When you automate
these commands, think about which audience your automation belongs to and
consume the appropriate signal for it.

Findings in a report are information, not necessarily problems. Some findings
describe conditions you will want to change; others describe conditions you
simply need to be aware of. Classify each finding you see: is this something
to fix, something to escalate, or something to record? Acting on a finding
without classifying it first is how evidence work goes wrong.

When a command refuses to run at all, that is qualitatively different from a
successful run whose report carries findings. A refusal means the tool could
not do its job with the inputs given — fix the inputs. A successful run with
findings means the tool did its job and is telling you something — listen to
it. Keeping this distinction clear in your head will save you significant
time.

## Working with reviewers

The projection exists to focus human review effort. When you hand a
projection to a reviewer, accompany it with context: what the pull request is
for, what you observed, and anything you noticed during capture that the
reviewer should know. The projection is a summary, and summaries serve their
readers best when the reader knows what question they are trying to answer.

If a reviewer asks for information the projection does not carry, do not
paraphrase from memory — go back to the underlying records and quote them.
The whole design of this surface is that every summary is backed by retained
material; use that property rather than working around it.

## Hygiene

Keep your capture directories organized and separate: one capture, one
directory, one manifest. Mixing files from different capture sessions in one
directory is a recipe for confusion about which bytes underlie which record.
Name things consistently, prefer explicit paths over clever relative ones,
and when in doubt, make a fresh directory rather than reusing an old one.

Never edit a captured file, for any reason, including "obvious" fixes like
correcting encoding or reformatting JSON. If a capture is wrong, capture
again. The value of retained evidence is that it is exactly what the provider
returned; an edited capture has no such value regardless of how minor the
edit. The same applies to the tool's outputs: if an output seems wrong,
re-run the command rather than adjusting the file.

Record what you do as you do it. A short log of the commands you ran, in
order, with a note of anything surprising, turns a debugging session from
archaeology into reading. Future operators — including future you — will be
grateful.

## When things go wrong

Work from the most recent message backwards: the tool's messages are specific,
and the most recent one usually names the immediate obstacle. Fix one thing at
a time and re-run. If you find yourself changing the same thing repeatedly,
stop — you are guessing, and the situation calls for reading rather than
guessing. Re-read the relevant output in full; the answer is usually already
on your screen.

If you exhaust the messages and remain stuck, capture the exact command, the
exact output, and the state of your directory, and escalate with those three
things. A well-formed escalation is answerable; "it doesn't work" is not.

Above all: never weaken your inputs to make a check pass, and never present a
derived summary as if it were the underlying evidence. The discipline is the
product. The commands enforce what they can, but the operator's care is what
makes the evidence trustworthy end to end.

## A note on automation

Automating these commands is encouraged once you understand them manually, and
premature automation is discouraged for the same reason: an automation encodes
an understanding, and encoding a misunderstanding merely makes the mistake
repeatable. Drive each command by hand at least once against a capture you
know well, read every field of its output, and only then wrap it in scripting.
When you do automate, make the automation transparent: log the exact command
lines, preserve the full reports rather than extracted fragments, and make the
failure path as well-defined as the success path. An automation that discards
the report body on failure has discarded exactly the material you will need.

Prefer small, single-purpose scripts over one script that does everything. A
script that captures, manifests, observes, projects, and gates in one motion
is convenient until the day one stage misbehaves, at which point the
convenience becomes opacity. Separate stages give you natural checkpoints at
which a human can read, verify, and decide — and those checkpoints are where
evidence discipline actually lives.

Review your automations periodically against the current tool version. Tools
evolve; a wrapper written against last month's behaviour can silently mask new
information the tool now provides, or silently depend on behaviour that has
been tightened. Reading the tool's own usage output after each upgrade, and
comparing it with what your scripts assume, is cheap insurance.

## Closing

None of the above is specific magic; it is the ordinary discipline of
evidence handling applied to one command family. Capture deliberately,
declare honestly, read what the tool tells you, keep tool failure distinct
from reported findings, never edit evidence, and keep humans in the loop at
the points where judgment is required. Operators who follow these practices
find the command family predictable and pleasant; operators who skip them
find it strict. The strictness is the point: it is what makes the output
worth retaining.

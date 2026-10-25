# Contributing

## Branches

`dev` is the integration branch and `main` is the release branch. Cut feature branches from `dev`; open changes against `dev`; merge `dev` into `main` only for a release. Never commit directly to `main`.

## Commits

Use Conventional Commits, imperative mood, with the area as the scope:

```
docs(ir): correct the node identity rule
feat(analysis): infer array extents from loop guards
fix(translate): preserve reverse member destruction order
```

The subject states the change; the body explains why it was needed and what evidence supports it. Wrap the body at around 72 characters. Keep a commit to one purpose. A breaking schema or CLI change is its own commit, so the contract change is easy to find.

Commits carry no co-author, generated-by or tool-attribution footers.

## Tags

Tags are annotated and the message starts with the tag name, matching the release history:

```
git tag -a v0.1.0 -m "v0.1.0: first working vertical slice"
```

Use `spec-vMAJOR.MINOR.PATCH` for a tagged state of the specification and `vMAJOR.MINOR.PATCH` for a verified implementation release. Tag only when the state is reproducible: the documents are consistent, or the code typechecks and its tests pass. Never move or delete a published tag.

## Code and documents

- Comments explain why, not what. Do not add decorative separator comments.
- Keep functions and modules named for the job they do.
- Put tests next to the code they cover in the project's idiomatic layout.
- A translation or analysis rule needs its rule ID, a worked example and a golden test before it is considered implemented.
- When a document is ambiguous, choose the recommended default from `docs/12-risks-decisions-open-questions.md`, record the choice, and continue.

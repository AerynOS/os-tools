This crate is an opinionated wrapper around the git2 crate, that allows us to work with a reduced API surface of the latter.
In fact, there are many repetitive tasks involving Git operations that we need to perform across our tooling.
We want a crate that focuses solely on these tasks.

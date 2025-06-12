# Contribution Guidelines

:+1::tada: First off, thanks for taking the time to contribute! :tada::+1:

We accept contributions in the following forms (non-exhaustive):

- **Bug reports**: A good bug report includes a succinct,
  reproducible context and the intended behavior.
- **Feature requests**: Should be followed by a thorough explanation of why
  the feature is important. Please note that not all feature requests will be
  accepted. If in doubt, please consider opening a discussion first.
- **Pull requests**: Clear and correct code with explanatory documentation
  and comments if necessary. If adding new functionality or fixing bugs,
  we expect accompanying tests. Pull requests will undergo a review process,
  and if accepted, the changes will be incorporated into the codebase.
- **Discussions**: Contributions that cannot be framed as bug reports
  or pull requests are good candidates for discussions. Please be polite
  and present civilized arguments in discussions.
- **Security Reports**: If you believe you have found a vulnerability,
  please provide details [here](mailto:security@alpenlabs.io) instead.

## Code of Conduct

All contributors are expected to show respect and courtesy to others.
To make clear what is expected, everyone contributing is required to conform to
the Code of Conduct.

We are dedicated to providing a welcoming and supportive environment for all people,
regardless of background or identity.
As such, we do not tolerate behaviour that is disrespectful to our contributors, developers, and users,
or that excludes, intimidates, or causes discomfort to others.
We do not tolerate discrimination or harassment based on characteristics that include,
but are not limited to, gender identity and expression, sexual orientation, disability,
physical appearance, body size, citizenship, nationality, ethnic or social origin, pregnancy,
familial status, veteran status, genetic information, religion or belief (or lack thereof),
membership of a national minority, property, age, education, socioeconomic status,
technical choices, and experience level.

### Expected Behaviour

All contributors are expected to show respect and courtesy to others.
All interactions should be professional regardless of platform:
either online or in-person.
In order to foster a positive and professional environment we encourage
the following kinds of behaviours:

- Use welcoming and inclusive language
- Be respectful of different viewpoints and experiences
- Gracefully accept constructive criticism
- Show courtesy and respect towards others

### Unacceptable Behaviour

Examples of unacceptable behaviour:

- written or verbal comments which have the effect of excluding people on
  the basis of membership of any specific group
- causing someone to fear for their safety, such as through stalking,
  following, or intimidation
- violent threats or language directed against another person
- the display of sexual or violent images
- unwelcome sexual attention
- nonconsensual or unwelcome physical contact
- sustained disruption of talks, events or communications
- insults or put downs
- sexist, racist, homophobic, transphobic, ableist, or exclusionary jokes
- incitement to violence, suicide, or self-harm
- continuing to initiate interaction (including photography or recording)
  with someone after being asked to stop
- publication of private communication without consent

## Development Tools

Please install the following tools in your development environment to make sure that
you can run the basic CI checks in your local environment:

- [`taplo`](https://taplo.tamasfe.dev/cli/installation/binary.html):
  used to lint and format `TOML` files.
- [`codespell`](https://github.com/codespell-project/codespell):
  used to check for common misspellings in code.
- [`cargo-nextest`](https://nexte.st):
  modern test runner for Rust.
- [`cargo-audit`](https://docs.rs/cargo-audit/latest/cargo_audit/):
  tool to check `Cargo.lock` files for security vulnerabilities.
- [`bitcoind`](https://bitcoin.org/en/download): to run the unit and functional tests.
- Functional test runner:
  to run functional tests, see instructions in its
  [`README.md`](./functional-tests/README.md).

## Locally running CI

Before you create a PR, make sure that all the required CI checks pass locally.
For your convenience, a `Makefile` recipe has been created which you can run via:

```bash
make pr # `make` should already be installed in most systems
```

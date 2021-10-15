pxls-rs
=======

An unfinished server implementation of [PxlsNetworking](https://github.com/BlankParenthesis/PxlsNetworking).
Note: that specification is evolving as this is worked on, so both are likely to change significantly.

Important missing features:
- Websocket authorization isn't compatible with the browser API specification.
- ~~No cooldown notifications.~~
- No permissions management.
- Basically no extensions are implemented.
  This is intentional in the case of some (like chat) but the intent is definitely to implement others in future.

Notable other issues:
- Startup could be way too slow currently (boards are reconstructed from database placements).
- A bunch of things require cleanup.
- Some features are only half implemented.
- There's probably a bunch of internal caching to do.
- There's probably a bunch of external HTTP caching info that should be revealed.
- Basically anything that's a TODO needs work.
- I'd like to make Clippy harsher and do some general cleanup.

*This code is currently not under any specific license.
If you wish to contribute to the development, contact me and I will likely add an open license.*
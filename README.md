pxls-rs
=======

An unfinished server implementation of [PxlsNetworking](https://github.com/BlankParenthesis/PxlsNetworking).
*Note: that specification is evolving as this is worked on, so both are likely to change significantly.*

Important missing features:
- ~~Websocket authorization isn't compatible with the browser API specification.~~
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

It's not all bad, here are some of the things that are currently better than the existing pxls implementation:
- Support for multiple simultaneous boards.
- Support for much larger boards through chunking.
- Board lifecycle management through API rather than restarts.
- Openid support.
- Partially transparent palette values.

And from a more development perspective:
- Database migrations.
- Leaner database structure.
- Smaller server scope (no need to worry about serving client files).
- A more sane and consistent API.

*This code is currently not under any specific license.
If you wish to contribute to the development, contact me and I will likely add an open license.*
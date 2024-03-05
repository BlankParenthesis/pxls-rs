pxls-server
===========

An unfinished server implementation of [PxlsNetworking](https://github.com/BlankParenthesis/PxlsNetworking).
*Note: that specification is evolving as this is worked on, so both are likely to change significantly.*

Important missing features:
- ~~Websocket authorization isn't compatible with the browser API specification.~~
- ~~No cooldown notifications.~~
- ~~No permissions management.~~
- Several important extensions are not yet implemented.
  
  A full list of which extensions are implemented:
	- [X] [authentication](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/authentication.md)
	- [X] [board_data_initial](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_data_initial.md)
	- [X] [board_data_mask](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_data_mask.md)
	- [X] [board_data_timestamps](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_data_timestamps.md)
	- [X] [board_lifecycle](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_lifecycle.md)
	- [ ] [board_moderation](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_moderation.md)
	- [ ] [board_notices](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_notices.md)
	- [ ] [board_undo](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_undo.md)
	- [ ] [cooldown_info](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/cooldown_info.md)
	- [ ] [factions](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/factions.md)
	- [ ] [list_filtering](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/list_filtering.md)
	- [ ] [placement_statistics](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/placement_statistics.md)
	- [ ] [reports](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/reports.md)
	- [X] [roles](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/roles.md)
	- [ ] [site_notices](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/site_notices.md)
	- [ ] [user_bans](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/user_bans.md)
	- [X] [user_count](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/user_count.md)
	- [X] [users](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/users.md)


Notable other issues:
- ~~Startup could be way too slow currently (boards are reconstructed from database placements).~~
  *Reconstruction cost can be mitigated with board sectors/chunking.*
- Some things require cleanup.
- There's probably a bunch of internal caching to do.
- There's probably a bunch of external HTTP caching info that should be revealed.
- Basically anything that's a TODO needs work.

It's not all bad, here are some of the things that are currently better than the existing pxls implementation:
- Support for multiple simultaneous boards.
- Support for much larger boards through chunking.
- Board lifecycle management through API rather than restarts.
- Openid support.
- Partially transparent palette values.

And from a more development perspective:
- Database migrations and a leaner database structure.
- Smaller server scope (no need to worry about serving client files).
- A more sane and consistent API.
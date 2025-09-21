pxls-rs (name pending)
===========

A big thank you to [CDawg](https://www.twitch.tv/cdawg) for funding this project and using it in the 2025 Connor canvas.

An in-progress reference server implementation of [PxlsNetworking](https://github.com/BlankParenthesis/PxlsNetworking).
*Note: that specification is evolving as this is worked on, so both are likely to change significantly.*

  A full list of which extensions are implemented:
	- [X] [authentication](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/authentication.md)
	- [X] [board_data_initial](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_data_initial.md)
	- [X] [board_data_mask](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_data_mask.md)
	- [X] [board_data_timestamps](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_data_timestamps.md)
	- [X] [board_lifecycle](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_lifecycle.md)
	- [X] [board_moderation](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_moderation.md)
	- [X] [board_notices](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_notices.md)
	- [X] [board_undo](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/board_undo.md)
	- [ ] [cooldown_info](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/cooldown_info.md)
	- [X] [factions](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/factions.md)
	- [X] [list_filtering](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/list_filtering.md)
	- [X] [placement_statistics](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/placement_statistics.md)
	- [X] [reports](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/reports.md)
	- [X] [roles](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/roles.md)
	- [X] [site_notices](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/site_notices.md)
	- [X] [user_bans](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/user_bans.md)
	- [X] [user_count](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/user_count.md)
	- [X] [users](https://github.com/BlankParenthesis/PxlsNetworking/blob/master/extensions/users.md)


Notable other issues:
- Some things require cleanup.
- There's probably a bunch of external HTTP caching info that should be revealed.
- Basically anything that's a TODO needs work.

It's not all bad, here are some of the things that are currently better than the existing pxls implementation:
- Support for multiple simultaneous boards.
- Support for much larger boards through chunking.
- Board lifecycle management through API rather than restarts.
- Openid support.
- Partially transparent palette values.
- Significant speed improvements and lower overall resource footprint

And from a more development perspective:
- Database migrations and a leaner database structure.
- A more sane and consistent API.

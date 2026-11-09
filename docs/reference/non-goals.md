# What rbx does not do

Deliberate omissions, with the reasoning, so "why not X?" is a link rather than a debate. The rule behind most of them: `rbx` reconciles a repository with Roblox, and anything that has no file to reconcile from is out of scope.

**Building the `.rbxl` from source.** That is [rojo](https://rojo.space)'s job, and it does it well. Composition is the design: rojo builds the place file, `rbx place upload` ships it. A tool that did both would be worse at each, and would have to keep pace with rojo's project format forever.

**An asset upload pipeline.** That is [asphalt](https://github.com/jackTabsCode/asphalt)'s job: hashing, uploading and generating references for images, sounds and models.

**Acting on a running game.** Servers, bans, data store entries, messages to live servers: none of it has a desired state a repository can hold, and a mistake costs player data rather than a retried deploy. A tool meant to run on every push should not be the one holding those commands.

**Managing API keys, opening Studio, downloading assets, switching Studio accounts.** Useful next to this tool, and none of them is a step in reconciling a repository with Roblox. Nothing here depends on a tool that does them.

**`destroy`.** The platform cannot really delete a universe, a badge or a game pass; it can archive some of them and refuses on others. A `destroy` that silently means "archive" is a command whose name lies at the worst possible moment, and the risk of getting that wrong is not worth what it buys today. Ask again if Roblox ships real deletion.

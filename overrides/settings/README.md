# Settings overrides

`.lua` files here replace LSB's settings files of the same name (`main.lua`, `map.lua`, `login.lua`, `network.lua`, `search.lua`, `logging.lua`). LSB fills in anything a file leaves out from its defaults, so a file only needs the values you change, for example `map.lua`:

```lua
xi = xi or {}
xi.settings = xi.settings or {}

xi.settings.map =
{
    EXP_RATE = 2.0,
}
```

Restart the servers to apply: `docker compose up -d --force-recreate connect search world map`.

Don't put SQL settings in `network.lua`: the stack sets those through the environment.

# Humanplane data in Terminal v0.1.1

This is the native Rust Terminal workspace with humanplane as its market-data provider. Start the humanplane backend and set `TR_HUMANPLANE_URL` to its base URL, without `/api`. The default is `http://127.0.0.1:8080`. For a remote backend, supply the URL when launching the Terminal executable or map the local port through an SSH tunnel.

In Terminal, enter `SEC ETH 5M` to search the active five-minute market and select an outcome. Other markets are searchable by name. Use `OB` for the selected outcome's order book. Each outcome is stored as its own `PM:<token>` market key. The native chart polls humanplane's price history and aggregates sampled prices into bars. The watchlist midpoint is sourced from actual displayed depth. No historical volume series is supplied by the price-history endpoint.

The public GitHub Actions runner builds the universal Mac disk image. The humanplane server is a separate service.

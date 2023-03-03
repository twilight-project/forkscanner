const { JsonRpc } = require("node-jsonrpc-client");

const client = new JsonRpc("http://my-coin-server:8339");

client.call("add_watched_addresses", {
    "add": [
        { "address": "bc1qm34lsc65zpw79lxes69zkqmk6ee3ewf0j77s3h", "watch_until": "2027-01-01T00:00:00Z" },
        { "address": "16xCNZEBEp424rZp27caZCqTqCW5hVBh8c", "watch_until": "2027-01-01T00:00:00Z" }
    ]
})
  .then((result) => {
    console.log("output", result.output);
  })
  .catch((err) => {
    console.error("Oops! Error code " + err.code + ": " + err.message);
  });
        

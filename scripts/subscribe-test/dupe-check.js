const WebSocket = require('websocket').w3cwebsocket;

var subscriptions = {}; 

const ws = new WebSocket('ws://localhost:8340');
ws.addEventListener('open', () => {
  console.log('Sending request');

  ws.send(JSON.stringify({
      jsonrpc: "2.0",
      id: "watched_address_checks",
      method: "watched_address_checks",
      params: {
          watch: [
              "bcrt1qsexvm8leel8afr3x66pzp6hdweuueqe56yq238"
          ],
          watch_until: "2030-09-30T00:00:00Z",
      }   
  }));
});


process.on('SIGINT', function () {
    console.log("Cancelling subscriptions");
    for ( const [key, id] of Object.entries(subscriptions)) {
        console.log(`Unsubscribe ${key} with id ${id}`);

        const method = `unsubscribe_${key}`;

        ws.send(JSON.stringify({
            jsonrpc: "2.0",
            id: method,
            method: method,
            params: { id: id },
        }));
    }   
    process.exit();
});


ws.addEventListener('message', (message) => {
  const obj = JSON.parse(message.data);

  if (obj.id !== undefined) {
      console.log('Subscription id: ', obj.result);
      subscriptions[obj.id] = obj.result;
  } else {
      if (obj.method == "watched_address_checks") {
          console.log(`Got watched address method`);
          let pras = new Set();

console.log(JSON.stringify(obj));
          //for (let p of obj.params) {
          //    let f = `${p.block}-${p.txid}-${p.sending}-${p.receiving}-${p.satoshis}`;
          //    console.log(`KEY: ${f}`);

          //    if (pras.has(f)) {
          //        console.log(`DUPE FOUND! ${f}`);
          //    } else {
          //        pras.add(f);
          //    }
          //}
      }
  }
});

console.log('Starting');

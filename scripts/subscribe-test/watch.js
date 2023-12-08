var req = {
 add: [
     { address: "cdef9ae998abe7d1c287d741ab9007de848294c0", watch_until: "2030-09-30T00:00:00.0Z" },
     { address: "db0bda0eed1402f76e4a34602928e3ad8238394c", watch_until: "2030-09-30T00:00:00.0Z" },
 ],
};

fetch("http://localhost:8339", {
  method: "POST",
  body: JSON.stringify({
      jsonrpc: "2.0",
      id: "1",
      method: "add_watched_addresses",
      params: req,
  }),
  headers: {
    "Content-type": "application/json"
  }
})
.then((response) => response.json())
.then((json) => console.log(json));

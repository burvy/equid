[out:json][timeout:120];
(
  node["highway"~"^(crossing|traffic_signals|stop|give_way)$"]({{BBOX}});
  node["crossing"]({{BBOX}});
  node["kerb"]({{BBOX}});
);
out body;

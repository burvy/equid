[out:json][timeout:180];
(
  nwr["amenity"~"^(waste_basket|bicycle_parking|shelter|fountain|post_box)$"]({{BBOX}});
  nwr["leisure"~"^(playground|dog_park|picnic_table|swimming_pool|fitness_station|pitch)$"]({{BBOX}});
  node["highway"~"^(crossing|street_lamp|bus_stop|traffic_signals)$"]({{BBOX}});
  way["footway"="crossing"]({{BBOX}});
  node["natural"="tree"]({{BBOX}});
  nwr["tourism"~"^(artwork|picnic_site)$"]({{BBOX}});
);
out tags center;

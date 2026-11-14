[out:json][timeout:120];
(
  node["highway"="street_lamp"]({{BBOX}});
  nwr["amenity"~"^(university|college)$"]({{BBOX}});
);
out body geom;

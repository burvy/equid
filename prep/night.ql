[out:json][timeout:120];
(
  node["highway"="street_lamp"](40.330,-74.690,40.370,-74.630);
  nwr["amenity"~"^(university|college)$"](40.330,-74.690,40.370,-74.630);
);
out body geom;

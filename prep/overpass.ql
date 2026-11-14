[out:json][timeout:180];
(
  way["highway"](40.330,-74.690,40.370,-74.630);
  nwr["amenity"~"^(bench|toilets|drinking_water|cafe|restaurant|fast_food|ice_cream|library|pharmacy|bicycle_parking|shelter)$"](40.330,-74.690,40.370,-74.630);
  nwr["leisure"~"^(park|garden|playground|nature_reserve)$"](40.330,-74.690,40.370,-74.630);
  nwr["tourism"~"^(viewpoint|museum|artwork|attraction|picnic_site)$"](40.330,-74.690,40.370,-74.630);
);
out body geom;

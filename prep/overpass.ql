[out:json][timeout:180];
(
  way["highway"]({{BBOX}});
  nwr["amenity"~"^(bench|toilets|drinking_water|cafe|restaurant|fast_food|ice_cream|library|pharmacy|bicycle_parking|shelter)$"]({{BBOX}});
  nwr["leisure"~"^(park|garden|playground|nature_reserve)$"]({{BBOX}});
  nwr["tourism"~"^(viewpoint|museum|artwork|attraction|picnic_site)$"]({{BBOX}});
);
out body geom;

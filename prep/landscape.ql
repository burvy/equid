[out:json][timeout:180];
(
  nwr["leisure"~"^(park|garden|nature_reserve|golf_course)$"]({{BBOX}});
  nwr["landuse"~"^(grass|forest|meadow|cemetery|recreation_ground|village_green)$"]({{BBOX}});
  nwr["natural"~"^(wood|scrub|grassland|wetland|heath|water)$"]({{BBOX}});
  nwr["waterway"~"^(river|canal|stream|riverbank)$"]({{BBOX}});
);
out body geom;

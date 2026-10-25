import { useEffect, useState } from "react";

import { cachedWeather, loadWeather, type WeatherCondition } from "./weather";

/**
 * The current condition for the connection's approximate location.
 *
 * The first state is read from the cache synchronously, so a recent condition is
 * present on the first paint. The request only starts after mount and updates
 * the greeting once the screen is already usable.
 */
export function useWeather(): WeatherCondition | null {
  const [condition, setCondition] = useState<WeatherCondition | null>(() => cachedWeather());

  useEffect(() => {
    let active = true;
    void loadWeather().then((next) => {
      if (active && next) setCondition(next);
    });
    return () => {
      active = false;
    };
  }, []);

  return condition;
}

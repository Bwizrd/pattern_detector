Feature: Supply and Demand Zone Detection

  Scenario: Detecting Supply Zones (Buy-Then-Sell)
    Given a series of candles on a price chart
    When there is a strong bullish candle followed by a bearish imbalance
    And the price returns to a zone created from the wick of the initial bullish candle
    Then a supply zone should be identified at the 50% zone line
    And the zone should be marked from the wick of the initial candle

  Scenario: Detecting Demand Zones (Sell-Then-Buy)
    Given a series of candles on a price chart
    When there is a strong bearish candle followed by a bullish imbalance
    And the price returns to a zone created from the wick of the initial bearish candle
    Then a demand zone should be identified at the 50% zone line
    And the zone should be marked from the wick of the initial candle
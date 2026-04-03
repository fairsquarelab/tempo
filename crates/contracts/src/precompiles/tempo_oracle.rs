pub use ITempoOracle::ITempoOracleErrors as TempoOracleError;

crate::sol! {
    #[derive(Debug, PartialEq, Eq)]
    #[sol(abi)]
    interface ITempoOracle {
        /// Returns the number of registered currencies.
        function currencyCount() external view returns (uint256);

        /// Returns the currency id at `index` (0 <= index < currencyCount).
        function registeredCurrencyAt(uint256 index) external view returns (uint32);

        /// Whether `currencyId` is registered.
        function isCurrencyRegistered(uint32 currencyId) external view returns (bool);

        /// Registers a currency id (ISO 4217 numeric, e.g. 410 for KRW). Caller becomes admin for registry ops if first call.
        function registerCurrency(uint32 currencyId) external;

        /// Unregisters a currency id.
        function unregisterCurrency(uint32 currencyId) external;

        /// Updates median prices from aggregated feeds. Restricted to block builder / system path.
        function setPrices(uint32[] calldata currencyIds, uint256[] calldata medians) external;

        /// One entry per registered currency (`currencyCount`); order in calldata may differ from registry order.
        /// Unregistered or duplicate `currencyId` in the same call reverts. Same block + same sender cannot
        /// call twice (reverts DuplicateOracleFeedUpdate). On success, values are stored as one pending row
        /// in registry order for `setPriceFeed` aggregation.
        struct OracleCurrencyPrice {
            uint32 currencyId;
            uint256 price;
        }

        function updatePriceFeed(OracleCurrencyPrice[] calldata updates) external;

        /// Reads pending feeds for the current block, checks quorum vs active validators, computes medians, writes `median_price`, clears pending for this block. Calldata-empty; caller must be block beneficiary (leader).
        function setPriceFeed() external;

        /// Returns the latest median price for `currencyId` (6 decimals, FxPrice encoding).
        function getOraclePrice(uint32 currencyId) external view returns (uint256);

        /// Returns all registered currency IDs.
        function getCurrencies() external view returns (uint32[] memory);

        /// Returns the cross rate: units of `base` per 1 `quote` (6-decimal fixed-point).
        /// Reverts CurrencyNotRegistered if either currency is not registered.
        /// Returns 0 if either median price has not been set yet.
        function getPairPrice(uint32 base, uint32 quote) external view returns (uint256);

        /// Basis points band used in `setPriceFeed` median filtering (`0` = disabled).
        function oracleMaxDeviationBps() external view returns (uint256);

        /// Quorum fraction for `setPriceFeed`: need ≥ ceil(active × num / den) feed rows.
        function oracleFeedThreshold() external view returns (uint256 num, uint256 den);

        /// Updates `oracle_max_deviation_bps`. Registry admin only.
        function setOracleMaxDeviationBps(uint32 newBps) external;

        /// Updates quorum numerator/denominator. Registry admin only; `den` must be non-zero, `num` in `(0, den]`.
        function setOracleFeedThreshold(uint256 num, uint256 den) external;

        error Unauthorized();
        error CurrencyNotRegistered();
        error ArraysLengthMismatch();
        error DuplicateOracleFeedUpdate();
        error InsufficientFeeds();
        error InvalidOracleParams();
    }
}

impl TempoOracleError {
    pub const fn unauthorized() -> Self {
        Self::Unauthorized(ITempoOracle::Unauthorized {})
    }

    pub const fn currency_not_registered() -> Self {
        Self::CurrencyNotRegistered(ITempoOracle::CurrencyNotRegistered {})
    }

    pub const fn arrays_length_mismatch() -> Self {
        Self::ArraysLengthMismatch(ITempoOracle::ArraysLengthMismatch {})
    }

    pub const fn duplicate_oracle_feed_update() -> Self {
        Self::DuplicateOracleFeedUpdate(ITempoOracle::DuplicateOracleFeedUpdate {})
    }

    pub const fn insufficient_feeds() -> Self {
        Self::InsufficientFeeds(ITempoOracle::InsufficientFeeds {})
    }

    pub const fn invalid_oracle_params() -> Self {
        Self::InvalidOracleParams(ITempoOracle::InvalidOracleParams {})
    }
}

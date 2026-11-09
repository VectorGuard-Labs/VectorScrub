"""
Registry for data broker detectors.
"""

from typing import Dict, List, Type

from .base import DataBrokerDetector


class DataBrokerRegistry:
    """
    Registry for all data broker detectors.

    Manages registration and retrieval of detector instances.
    """

    _detectors: Dict[str, Type[DataBrokerDetector]] = {}

    @classmethod
    def register(cls, detector_class: Type[DataBrokerDetector]):
        """
        Register a data broker detector.

        Args:
            detector_class: Detector class to register
        """
        # Instantiate to get broker name
        instance = detector_class()
        broker_name = instance.broker_name

        cls._detectors[broker_name] = detector_class

    @classmethod
    def get_detector(cls, broker_name: str) -> DataBrokerDetector:
        """
        Get a detector instance by broker name.

        Args:
            broker_name: Name of the broker

        Returns:
            Detector instance

        Raises:
            KeyError: If broker not registered
        """
        if broker_name not in cls._detectors:
            raise KeyError(f"No detector registered for broker: {broker_name}")

        return cls._detectors[broker_name]()

    @classmethod
    def get_all_detectors(cls) -> List[DataBrokerDetector]:
        """
        Get instances of all registered detectors.

        Returns:
            List of detector instances
        """
        return [detector_class() for detector_class in cls._detectors.values()]

    @classmethod
    def list_brokers(cls) -> List[str]:
        """
        Get list of all registered broker names.

        Returns:
            List of broker names
        """
        return list(cls._detectors.keys())


def register_detector(detector_class: Type[DataBrokerDetector]):
    """
    Decorator to register a data broker detector.

    Usage:
        @register_detector
        class MyDetector(DataBrokerDetector):
            ...
    """
    DataBrokerRegistry.register(detector_class)
    return detector_class

from abc import ABC, abstractmethod


class GreetingStrategy(ABC):
    @abstractmethod
    def greeting(self, name: str) -> str: ...


class PolishGreeting(GreetingStrategy):
    def greeting(self, name: str) -> str:
        return f"Cześć, {name}!"


_REGISTRY: dict[str, type[GreetingStrategy]] = {"pl": PolishGreeting}


class GreetingFactory:
    @staticmethod
    def create(language: str = "pl") -> GreetingStrategy:
        return _REGISTRY[language]()


def greet(name):
    return GreetingFactory.create().greeting(name)

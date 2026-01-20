# IntentGym API Reference

## Core Components

### Runner (`intentgym.core.runner`)
The main orchestrator.

```python
from intentgym.core.config import RunConfig
from intentgym.core.runner import BenchmarkRunner

config = RunConfig.from_yaml("my_config.yaml")
runner = BenchmarkRunner(config)
results = runner.run()
```

### Agents (`intentgym.core.agent`)
Base class for all agents.

```python
class Agent(ABC):
    @abstractmethod
    def decide(self, state: AgentState, observation: OrynObservation) -> AgentAction:
        """Decide next action."""
        
    def update(self, state: AgentState, action: AgentAction, result: OrynResult):
        """Update internal state/memory."""
```

### LLM Provider (`intentgym.core.llm`)
Interface for Language Models.

```python
class LLMProvider(ABC):
    def complete(self, messages: List[Dict[str, str]]) -> LLMResponse:
        """Generate completion."""
```

## Extending IntentGym

### Adding a New Benchmark
Inherit from `intentgym.benchmarks.base.Benchmark`.

```python
from intentgym.benchmarks.base import Benchmark, Task, Evaluation

class MyNewBenchmark(Benchmark):
    def load_tasks(self, subset: str = "all") -> List[Task]:
        # Return list of tasks
        pass
        
    def evaluate(self, task: Task, oryn: OrynInterface) -> Evaluation:
        # Check if goal state reached
        pass
```

### Adding a New Agent
Inherit from `intentgym.core.agent.Agent`.

```python
from intentgym.core.agent import Agent, AgentState, AgentAction

class MyCustomAgent(Agent):
    def decide(self, state, observation):
        # Your custom logic here
        return AgentAction(command="click 1", reasoning="...")
```

### Adding a Framework Adapter
Use `FrameworkAdapter` to wrap external libraries.

```python
from intentgym.adapters.base import FrameworkAdapter

class MyAdapter(FrameworkAdapter):
    def step(self, state, observation):
        # Translate IntentGym observation to external framework
        # Run external agent
        # Translate result back to AgentAction
        return action
```

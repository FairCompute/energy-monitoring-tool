from emt import power_groups
from emt.power_groups import PowerGroup
from tabulate import tabulate


class PGUtils:
    def __init__(
        self,
    ):
        self.powergroup_types = []
        self.available_powergroups = []
        self.pg_objs = []

    def get_pg_types(self, module):
        candidates = [
            getattr(module, name)
            for name in dir(module)
            if isinstance(getattr(module, name), type)
        ]
        pg_types = filter(lambda x: issubclass(x, PowerGroup), candidates)
        return list(pg_types)

    def get_available_pgs(self):
        """
        Get available powergroups from the list of powergroups (.py files)
        """
        self.powergroup_types = self.get_pg_types(power_groups)
        # check for available power_groups
        self.available_powergroups = list(
            filter(lambda x: x.is_available(), self.powergroup_types)
        )
        # instantiate only available powergroups
        self.pg_objs = [pgt() for pgt in self.available_powergroups]
        return self.pg_objs

    def get_pg_table(self):
        """
        Get powergroup info in a tabular format
        """
        table = []
        headers = ["Devices", "Available", "Tracked"]
        for pg in self.powergroup_types:
            if pg.__name__ != "PowerGroup":  # ignore the base class
                table.append(
                    [
                        pg.__name__,
                        "Yes",
                        "Tracked @ 10Hz" if pg.is_available() else "No",
                    ]
                )

        return tabulate(table, headers, tablefmt="pretty")
